#!/usr/bin/env python3
"""Build Strata's source RPM from `packaging/rpm/strata.spec.in`.

Needs git, cargo, cargo-vendor-filterer, cargo2rpm, and rpmbuild (Fedora:
`dnf install git cargo-rpm-macros cargo-vendor-filterer rpm-build`).
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.parse

from update_aur import PackagingError, preview_release_version, render

ROOT = pathlib.Path(__file__).resolve().parent.parent
TEMPLATE = ROOT / "packaging" / "rpm" / "strata.spec.in"
VENDOR_PLATFORMS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")
# Compiled into the binary; see THIRD_PARTY_LICENSES.md.
ASSET_LICENSES = ("Apache-2.0", "ISC", "MIT", "OFL-1.1")
# unrar_sys bundles the UnRAR source, whose license Fedora and COPR do not allow.
FORBIDDEN_CRATES = ("unrar", "unrar_sys")
STUB_FILES = {"Cargo.toml", ".cargo-checksum.json", "src/lib.rs"}
VENDOR_GIT_CONFIG_NAME = "vendor-git-config.toml"
VENDOR_CONFIG = """\
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"

[net]
offline = true
"""


def rpm_version(release_version: str) -> str:
    """Uses `~` so RPM sorts a prerelease before its final release."""
    core, _, prerelease = preview_release_version(release_version).partition("-")
    return f"{core}~{prerelease}" if prerelease else core


def build_kind(release_version: str) -> str:
    _, _, prerelease = preview_release_version(release_version).partition("-")
    return prerelease.split(".", 1)[0] if prerelease else "stable"


def parse_license_summary(summary: str) -> list[str]:
    """Reads `cargo2rpm license-summary`, which prints `# `-prefixed expressions between markers."""
    expressions = []
    for line in summary.splitlines():
        expression = line.strip().lstrip("#").strip()
        if expression and "LICENSE SUMMARY" not in expression:
            expressions.append(expression)
    return expressions


def wraps_whole(expression: str) -> bool:
    if not (expression.startswith("(") and expression.endswith(")")):
        return False
    depth = 0
    for character in expression[:-1]:
        depth += {"(": 1, ")": -1}.get(character, 0)
        if depth == 0:
            return False
    return True


def and_terms(expression: str) -> list[str]:
    """Splits an SPDX expression on its top-level `AND` operators."""
    terms, depth, start = [], 0, 0
    for index, character in enumerate(expression):
        depth += {"(": 1, ")": -1}.get(character, 0)
        if depth == 0 and expression.startswith(" AND ", index):
            terms.append(expression[start:index])
            start = index + len(" AND ")
    terms.append(expression[start:])
    unwrapped = []
    for term in (term.strip() for term in terms):
        while wraps_whole(term):
            term = term[1:-1].strip()
        if term:
            unwrapped.append(term)
    return unwrapped


def license_term(term: str) -> str:
    compound = " OR " in term or " AND " in term
    if compound and "(" not in term and " AND " not in term:
        term = " OR ".join(sorted(term.split(" OR "), key=str.casefold))
    return f"({term})" if compound else term


def license_expression(crate_licenses: list[str]) -> str:
    terms = set(ASSET_LICENSES)
    for expression in crate_licenses:
        terms.update(license_term(term) for term in and_terms(expression.strip()))
    return " AND ".join(sorted(terms, key=lambda term: term.strip("()").casefold()))


def is_stub(crate: pathlib.Path) -> bool:
    """Whether cargo-vendor-filterer replaced this crate with an empty stub."""
    files = {path.relative_to(crate).as_posix() for path in crate.rglob("*") if path.is_file()}
    return files == STUB_FILES and (crate / "src" / "lib.rs").stat().st_size == 0


def parse_git_source(source_url: str) -> dict[str, str | None]:
    """Splits a Cargo.lock `git+<url>?rev=...#<commit>` source into Cargo config fields."""
    base, _, commit = source_url.partition("#")
    url_part = base.removeprefix("git+")
    parsed = urllib.parse.urlsplit(url_part)
    query = urllib.parse.parse_qs(parsed.query)
    git_url = urllib.parse.urlunsplit((parsed.scheme, parsed.netloc, parsed.path, "", ""))
    return {
        "source": base,
        "url_part": url_part,
        "git": git_url,
        "rev": query.get("rev", [None])[0],
        "branch": query.get("branch", [None])[0],
        "tag": query.get("tag", [None])[0],
        "commit": commit or None,
    }


def _registry_git_collision(
    name: str, version: str, entries: list[dict]
) -> tuple[str, str, str, str] | None:
    if len(entries) <= 1:
        return None
    git_entries = [entry for entry in entries if entry["source"].startswith("git+")]
    reg_entries = [entry for entry in entries if entry["source"].startswith("registry+")]
    if len(git_entries) == 1 and len(reg_entries) == 1:
        return (name, version, git_entries[0]["source"], reg_entries[0].get("checksum", ""))
    return None


def lockfile_git_sources(
    lockfile: dict,
) -> tuple[dict[str, set[tuple[str, str]]], list[tuple[str, str, str, str]]]:
    """Returns git sources mapped to their `(name, version)` pairs and crates.io collisions."""
    by_package: dict[tuple[str, str], list[dict]] = {}
    git_sources: dict[str, set[tuple[str, str]]] = {}
    for package in lockfile.get("package", ()):
        source = package.get("source")
        if not source:
            continue
        key = (package["name"], package["version"])
        by_package.setdefault(key, []).append(package)
        if source.startswith("git+"):
            git_sources.setdefault(source, set()).add(key)

    collisions = [
        collision
        for (name, version), entries in sorted(by_package.items())
        if (collision := _registry_git_collision(name, version, entries)) is not None
    ]
    return git_sources, collisions


def git_vendor_config(git_sources: list[str]) -> str:
    """Builds `.cargo/config.toml` stanzas replacing each git source with `vendor-git/<index>`."""
    blocks = []
    for index, source_url in enumerate(sorted(git_sources)):
        info = parse_git_source(source_url)
        lines = [f'[source."{info["source"]}"]', f'git = "{info["git"]}"']
        for selector in ("rev", "branch", "tag"):
            if info[selector]:
                lines.append(f'{selector} = "{info[selector]}"')
        lines.append(f'replace-with = "vendored-git-{index}"')
        lines.append("")
        lines.append(f"[source.vendored-git-{index}]")
        lines.append(f'directory = "vendor-git/{index}"')
        blocks.append("\n".join(lines))
    return ("\n\n".join(blocks) + "\n") if blocks else ""


def vendor_config(git_config: str = "") -> str:
    return f"{VENDOR_CONFIG}\n{git_config}" if git_config else VENDOR_CONFIG


def patch_manifest_for_vendoring(
    manifest: str, collisions: list[tuple[str, str, str, str]]
) -> str:
    """Temporarily routes colliding crates.io names to their git source so `cargo vendor` runs."""
    if not collisions:
        return manifest
    lines = []
    for name, _, git_source, _ in collisions:
        info = parse_git_source(git_source)
        fields = [f'git = "{info["git"]}"']
        for selector in ("rev", "branch", "tag"):
            if info[selector]:
                fields.append(f'{selector} = "{info[selector]}"')
        lines.append(f"{name} = {{ {', '.join(fields)} }}")
    header = "" if "[patch.crates-io]" in manifest else "\n[patch.crates-io]"
    return f"{manifest.rstrip()}\n{header}\n" + "\n".join(lines) + "\n"


def format_stub_entry(name: str, version: str, git_source: str | None = None) -> str:
    if not git_source:
        return f"{name} v{version}"
    info = parse_git_source(git_source)
    short = (info["commit"] or "")[:8]
    return f"{name} v{version} ({info['url_part']}#{short})"


def inspect_vendor_directory(
    directory: pathlib.Path, git_source: str | None = None
) -> tuple[list[str], list[str]]:
    stubs = []
    forbidden = []
    for crate in sorted(path for path in directory.iterdir() if path.is_dir()):
        package = tomllib.loads((crate / "Cargo.toml").read_text())["package"]
        if is_stub(crate):
            stubs.append(format_stub_entry(package["name"], package["version"], git_source))
        elif package["name"] in FORBIDDEN_CRATES:
            forbidden.append(package["name"])
    return stubs, forbidden


def inspect_vendor(
    vendor: pathlib.Path, git_dirs: dict[pathlib.Path, str] | None = None
) -> tuple[list[str], list[str]]:
    """Returns stubbed crates in `cargo-vendor.txt` format and any forbidden crate with sources."""
    stubs, forbidden = inspect_vendor_directory(vendor)
    for git_dir, git_source in sorted((git_dirs or {}).items()):
        git_stubs, git_forbidden = inspect_vendor_directory(git_dir, git_source)
        stubs.extend(git_stubs)
        forbidden.extend(git_forbidden)
    return sorted(stubs), sorted(forbidden)


def run(
    arguments: list[str],
    cwd: pathlib.Path | None = None,
    env: dict[str, str] | None = None,
) -> str:
    result = subprocess.run(
        arguments, cwd=cwd, env=env, check=True, capture_output=True, text=True
    )
    return result.stdout


def require_updater_switch(source: pathlib.Path, tag: str) -> None:
    """Older sources ignore `STRATA_UPDATER`, so their RPM would ship the in-app updater."""
    candidates = [source / "build.rs", *source.glob("crates/*/build.rs")]
    build_scripts = [script for script in candidates if script.is_file()]
    if not any("STRATA_UPDATER" in script.read_text() for script in build_scripts):
        raise PackagingError(f"{tag} predates STRATA_UPDATER and cannot be packaged as an RPM")


def split_git_crates(
    source: pathlib.Path, git_sources: dict[str, set[tuple[str, str]]]
) -> dict[pathlib.Path, str]:
    """Moves each git source's crates out of `vendor/` into `vendor-git/<index>/`."""
    git_dirs: dict[pathlib.Path, str] = {}
    vendor = source / "vendor"
    for index, git_source in enumerate(sorted(git_sources)):
        destination = source / "vendor-git" / str(index)
        destination.mkdir(parents=True)
        git_dirs[destination] = git_source
        members = git_sources[git_source]
        for crate in sorted(path for path in vendor.iterdir() if path.is_dir()):
            package = tomllib.loads((crate / "Cargo.toml").read_text())["package"]
            if (package["name"], package["version"]) in members:
                shutil.move(str(crate), str(destination / crate.name))
    return git_dirs


def vendor_colliding_registry_crates(
    source: pathlib.Path, collisions: list[tuple[str, str, str, str]]
) -> None:
    """Vendors each crates.io package that shared a name and version with a git package."""
    with tempfile.TemporaryDirectory(prefix="strata-registry-vendor-") as scratch:
        work = pathlib.Path(scratch)
        for index, (name, version, _, expected_checksum) in enumerate(collisions):
            stub = work / f"crate-{index}"
            (stub / "src").mkdir(parents=True)
            (stub / "src" / "lib.rs").write_text("")
            (stub / "Cargo.toml").write_text(
                '[package]\nname = "strata-vendor-stub"\nversion = "0.1.0"\nedition = "2021"\n\n'
                f'[dependencies]\n{name} = "={version}"\n'
            )
            shutil.copy2(source / "Cargo.lock", stub / "Cargo.lock")
            run(["cargo", "vendor", "--offline", "--versioned-dirs", "vendor"], cwd=stub)
            vendored = stub / "vendor" / f"{name}-{version}"
            if not vendored.is_dir():
                raise PackagingError(f"failed to vendor crates.io package {name} {version}")
            if expected_checksum:
                actual = json.loads((vendored / ".cargo-checksum.json").read_text()).get("package")
                if actual != expected_checksum:
                    raise PackagingError(f"checksum mismatch vendoring {name} {version}")
            destination = source / "vendor" / f"{name}-{version}"
            if destination.exists():
                raise PackagingError(f"vendor directory already contains {destination.name}")
            shutil.copytree(vendored, destination)


def run_vendor_filterer(
    source: pathlib.Path,
    platforms: list[str],
    collisions: list[tuple[str, str, str, str]],
) -> None:
    command = [
        "cargo",
        "vendor-filterer",
        "--offline",
        "--no-default-features",
        "--keep-dep-kinds=no-dev",
        *platforms,
        "vendor",
    ]
    if not collisions:
        run(command, cwd=source)
        return
    with tempfile.TemporaryDirectory(prefix="strata-vendor-scratch-") as scratch:
        staged = pathlib.Path(scratch) / "source"
        shutil.copytree(source, staged)
        manifest = (staged / "Cargo.toml").read_text()
        (staged / "Cargo.toml").write_text(patch_manifest_for_vendoring(manifest, collisions))
        run(["cargo", "metadata", "--offline", "--format-version", "1"], cwd=staged)
        run(command, cwd=staged)
        shutil.move(str(staged / "vendor"), str(source / "vendor"))


def verify_offline_metadata(source: pathlib.Path, git_config: str) -> None:
    """Confirms the unmodified source and lockfile resolve offline from the vendored dirs alone."""
    config = source / ".cargo" / "config.toml"
    config.parent.mkdir(exist_ok=True)
    config.write_text(vendor_config(git_config))
    with tempfile.TemporaryDirectory(prefix="strata-empty-cargo-home-") as empty_home:
        env = {**os.environ, "CARGO_HOME": empty_home}
        run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"], cwd=source, env=env)


def _prepare_git_sources(
    source: pathlib.Path,
    git_sources: dict[str, set[tuple[str, str]]],
    collisions: list[tuple[str, str, str, str]],
) -> tuple[dict[pathlib.Path, str], str]:
    if not git_sources:
        return {}, ""
    git_dirs = split_git_crates(source, git_sources)
    if collisions:
        vendor_colliding_registry_crates(source, collisions)
    git_config = git_vendor_config(list(git_sources))
    (source / VENDOR_GIT_CONFIG_NAME).write_text(git_config)
    return git_dirs, git_config


def _require_unchanged_lockfile(source: pathlib.Path, expected: bytes) -> None:
    if (source / "Cargo.lock").read_bytes() != expected:
        raise PackagingError("Cargo.lock is out of date with Cargo.toml in the packaged source")


def vendor_crates(source: pathlib.Path, archive: pathlib.Path, stub_list: pathlib.Path) -> None:
    platforms = [f"--platform={platform}" for platform in VENDOR_PLATFORMS]
    lockfile_bytes = (source / "Cargo.lock").read_bytes()
    run(["cargo", "fetch", "--locked"], cwd=source)
    # The offline build uses the archived lockfile, so vendoring must not resolve anew.
    _require_unchanged_lockfile(source, lockfile_bytes)
    git_sources, collisions = lockfile_git_sources(tomllib.loads(lockfile_bytes.decode()))
    run_vendor_filterer(source, platforms, collisions)
    _require_unchanged_lockfile(source, lockfile_bytes)

    git_dirs, git_config = _prepare_git_sources(source, git_sources, collisions)
    verify_offline_metadata(source, git_config)
    stubs, forbidden = inspect_vendor(source / "vendor", git_dirs)
    if forbidden:
        raise PackagingError(f"vendored sources include non-free crates: {forbidden}")
    stub_list.write_text("".join(f"{stub}\n" for stub in stubs))

    members = ["vendor", "vendor-git", VENDOR_GIT_CONFIG_NAME] if git_sources else ["vendor"]
    run(
        [
            "tar",
            "--sort=name",
            "--mtime=@0",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "-czf",
            str(archive),
            *members,
        ],
        cwd=source,
    )


def crate_licenses(source: pathlib.Path) -> list[str]:
    git_config_path = source / VENDOR_GIT_CONFIG_NAME
    git_config = git_config_path.read_text() if git_config_path.is_file() else ""
    config = source / ".cargo" / "config.toml"
    config.parent.mkdir(exist_ok=True)
    config.write_text(vendor_config(git_config))
    summary = run(
        ["cargo2rpm", "--path", "Cargo.toml", "license-summary", "--no-default-features"],
        cwd=source,
    )
    return parse_license_summary(summary)


def build_srpm(arguments: argparse.Namespace) -> pathlib.Path:
    version = arguments.tag.strip().removeprefix("v")
    tag = f"v{version}"
    package_version = rpm_version(version)
    ref = arguments.ref or tag
    commit = run(["git", "rev-parse", "--short=12", f"{ref}^{{commit}}"], cwd=ROOT).strip()
    name = f"strata-{version}"
    arguments.outdir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="strata-srpm-") as scratch:
        work = pathlib.Path(scratch)
        sources = work / "SOURCES"
        sources.mkdir()
        archive = sources / f"{name}.tar.gz"
        run(
            ["git", "archive", "--format=tar.gz", f"--prefix={name}/", "-o", str(archive), commit],
            cwd=ROOT,
        )
        with tarfile.open(archive) as tree:
            tree.extractall(work, filter="data")
        source = work / name
        require_updater_switch(source, tag)
        vendor_crates(
            source, sources / f"{name}-vendor.tar.gz", sources / f"{name}-vendor-stubs.txt"
        )

        spec = sources / "strata.spec"
        spec.write_text(
            render(
                TEMPLATE.read_text(),
                {
                    "VERSION": package_version,
                    "RELEASE": str(arguments.release),
                    "TAG": tag,
                    "TAG_VERSION": version,
                    "BUILD_KIND": build_kind(version),
                    "COMMIT": commit,
                    "LICENSE": license_expression(crate_licenses(source)),
                    "CHANGELOG_DATE": arguments.date.strftime("%a %b %d %Y"),
                },
            )
        )
        output = run(
            [
                "rpmbuild",
                "-bs",
                "--define",
                f"_sourcedir {sources}",
                "--define",
                f"_srcrpmdir {arguments.outdir.resolve()}",
                str(spec),
            ]
        )
    match = re.search(r"^Wrote: (.+\.src\.rpm)$", output, flags=re.MULTILINE)
    if match is None:
        raise PackagingError(f"rpmbuild did not report a source RPM:\n{output}")
    return pathlib.Path(match.group(1))


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tag", required=True, help="release tag the package identifies as, e.g. v1.2.1")
    parser.add_argument("--ref", help="git revision to package (default: the tag)")
    parser.add_argument("--release", type=int, default=1, help="RPM release number (default: 1)")
    parser.add_argument("--outdir", type=pathlib.Path, default=ROOT / "target" / "rpm")
    parser.add_argument(
        "--date",
        type=datetime.date.fromisoformat,
        default=datetime.datetime.now(datetime.UTC).date(),
        help="changelog date, YYYY-MM-DD (default: today, UTC)",
    )
    arguments = parser.parse_args(argv)
    if arguments.release < 1:
        parser.error("--release must be at least 1")
    return arguments


def main(argv: list[str]) -> int:
    arguments = parse_args(argv)
    try:
        print(build_srpm(arguments))
    except PackagingError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    except subprocess.CalledProcessError as error:
        print(f"error: {' '.join(error.cmd)} failed:\n{error.stderr}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
