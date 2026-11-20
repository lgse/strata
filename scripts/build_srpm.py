#!/usr/bin/env python3
"""Build Strata's source RPM from `packaging/rpm/strata.spec.in`.

Needs git, cargo, cargo-vendor-filterer, cargo2rpm, and rpmbuild (Fedora:
`dnf install git cargo-rpm-macros cargo-vendor-filterer rpm-build`).
"""

from __future__ import annotations

import argparse
import datetime
import pathlib
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib

from update_aur import PackagingError, preview_release_version, render

ROOT = pathlib.Path(__file__).resolve().parent.parent
TEMPLATE = ROOT / "packaging" / "rpm" / "strata.spec.in"
VENDOR_PLATFORMS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")
# Compiled into the binary; see THIRD_PARTY_LICENSES.md.
ASSET_LICENSES = ("Apache-2.0", "ISC", "MIT", "OFL-1.1")
# unrar_sys bundles the UnRAR source, whose license Fedora and COPR do not allow.
FORBIDDEN_CRATES = ("unrar", "unrar_sys")
STUB_FILES = {"Cargo.toml", ".cargo-checksum.json", "src/lib.rs"}
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


def inspect_vendor(vendor: pathlib.Path) -> tuple[list[str], list[str]]:
    """Returns the stubbed crates as `name vVERSION` lines and any forbidden crate with sources."""
    stubs = []
    forbidden = []
    for crate in sorted(path for path in vendor.iterdir() if path.is_dir()):
        package = tomllib.loads((crate / "Cargo.toml").read_text())["package"]
        stub = is_stub(crate)
        if stub:
            stubs.append(f"{package['name']} v{package['version']}")
        elif package["name"] in FORBIDDEN_CRATES:
            forbidden.append(package["name"])
    return stubs, forbidden


def run(arguments: list[str], cwd: pathlib.Path | None = None) -> str:
    result = subprocess.run(arguments, cwd=cwd, check=True, capture_output=True, text=True)
    return result.stdout


def require_updater_switch(source: pathlib.Path, tag: str) -> None:
    """Older sources ignore `STRATA_UPDATER`, so their RPM would ship the in-app updater."""
    build_scripts = [source / "build.rs", *source.glob("crates/*/build.rs")]
    if not any("STRATA_UPDATER" in script.read_text() for script in build_scripts):
        raise PackagingError(f"{tag} predates STRATA_UPDATER and cannot be packaged as an RPM")


def vendor_crates(source: pathlib.Path, archive: pathlib.Path, stub_list: pathlib.Path) -> None:
    platforms = [f"--platform={platform}" for platform in VENDOR_PLATFORMS]
    lockfile = (source / "Cargo.lock").read_bytes()
    run(
        [
            "cargo",
            "vendor-filterer",
            "--no-default-features",
            "--keep-dep-kinds=no-dev",
            *platforms,
            "vendor",
        ],
        cwd=source,
    )
    # The offline build uses the archived lockfile, so vendoring must not resolve anew.
    if (source / "Cargo.lock").read_bytes() != lockfile:
        raise PackagingError("Cargo.lock is out of date with Cargo.toml in the packaged source")
    stubs, forbidden = inspect_vendor(source / "vendor")
    if forbidden:
        raise PackagingError(f"vendored sources include non-free crates: {forbidden}")
    stub_list.write_text("".join(f"{stub}\n" for stub in stubs))
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
            "vendor",
        ],
        cwd=source,
    )


def crate_licenses(source: pathlib.Path) -> list[str]:
    config = source / ".cargo" / "config.toml"
    config.parent.mkdir(exist_ok=True)
    config.write_text(VENDOR_CONFIG)
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
