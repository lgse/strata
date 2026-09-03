#!/usr/bin/env python3
"""Render the AUR packages in `packaging/aur/` for a published release.

Both packages are generated from the single template
`packaging/aur/PKGBUILD.in`: `strata-bin` tracks stable releases and
`strata-rc-bin` tracks the newest release candidate. The AUR requires a
self-contained `PKGBUILD`, so the rendered files are committed rather than
sourced from the template at build time.

Nightly releases are deliberately not packaged. A nightly's download URL
carries its own tag, and `makepkg` fetches `source` before `pkgver()` runs,
so a package cannot discover the newest nightly and download it in the same
build; pinning each one instead would mean an AUR push per nightly. Nightly
users install manually and update in-app.

Checksums are read from the `.sha256` files published alongside each release
archive, so a mistyped version fails loudly instead of pinning a wrong digest.

Each channel is rendered only when its own flag is passed. Rendering the RC
package from `--stable` would let a stable release silently roll the RC
package's `pkgver` backwards past a newer release candidate.

Usage:

    python3 scripts/update_aur.py --stable 0.7.0
    python3 scripts/update_aur.py --stable 0.7.0 --rc 0.7.1-rc.2
    python3 scripts/update_aur.py --rc 0.8.0-rc.1

`.SRCINFO` is regenerated with `makepkg --printsrcinfo`, which requires Arch's
pacman tooling; pass `--skip-srcinfo` on a non-Arch machine and regenerate it
before publishing. See `docs/packaging.md` for the full release procedure.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys
import urllib.error
import urllib.request

REPOSITORY_URL = "https://github.com/lgse/strata"
TARGETS = ("x86_64", "aarch64")

RELEASE_VERSION_PATTERN = re.compile(
    r"^(?P<core>\d+\.\d+\.\d+)(?:-(?P<prerelease>[0-9A-Za-z.]+))?$"
)

PACKAGES = {
    "strata-bin": {
        "channel": "stable",
        "alternate": "strata-rc-bin",
        "description": "A fast, keyboard-first file manager for Linux",
    },
    "strata-rc-bin": {
        "channel": "rc",
        "alternate": "strata-bin",
        "description": (
            "A fast, keyboard-first file manager for Linux (release candidates)"
        ),
    },
}


class PackagingError(ValueError):
    """Raised for an invalid version, a missing release, or a bad checksum."""


def package_version(release_version: str) -> str:
    """Converts a release version into a valid `pkgver`.

    `pkgver` forbids `-` and `:` but allows `.`, so only the hyphen
    separating the prerelease is removed: `0.8.0-rc.1` becomes `0.8.0rc.1`.

    Dots are deliberately kept. `vercmp` splits on non-alphanumerics and
    compares segment by segment, so dropping them would concatenate a
    prerelease's components into one number -- `0.8.0-nightly.20260901.2`
    would mangle to `0.8.0nightly202609012`, which orders *above*
    `0.8.0nightly20260902`. Keeping the separator makes every component its
    own segment, which orders correctly for every form of the tag grammar:
    `0.8.0rc.1 < 0.8.0rc.2 < 0.8.0rc.10 < 0.8.0`.
    """
    version = release_version.strip().removeprefix("v")
    match = RELEASE_VERSION_PATTERN.match(version)
    if match is None:
        raise PackagingError(
            f"release version must be major.minor.patch[-prerelease]: {release_version!r}"
        )
    prerelease = match.group("prerelease")
    if prerelease is None:
        return match.group("core")
    return match.group("core") + prerelease


def release_version(version: str) -> str:
    """Normalizes a release version, rejecting anything the tag grammar forbids."""
    normalized = version.strip().removeprefix("v")
    package_version(normalized)
    return normalized


def archive_name(version: str, target: str) -> str:
    return f"strata-{version}-{target}-unknown-linux-gnu.tar.gz"


def checksum_url(version: str, target: str) -> str:
    return (
        f"{REPOSITORY_URL}/releases/download/v{version}/{archive_name(version, target)}.sha256"
    )


def parse_checksum(contents: str, expected_archive: str) -> str:
    """Reads the digest out of a `sha256sum` line, checking the filename.

    The filename check is what makes a wrong `--stable`/`--preview` pairing
    fail here rather than silently pinning one release's digest to another
    release's URL.
    """
    for line in contents.splitlines():
        parts = line.split()
        if len(parts) != 2:
            continue
        digest, name = parts
        if name.lstrip("*") != expected_archive:
            raise PackagingError(
                f"checksum file names {name!r}, expected {expected_archive!r}"
            )
        if not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise PackagingError(f"not a sha256 digest: {digest!r}")
        return digest
    raise PackagingError(f"no checksum line found for {expected_archive!r}")


def fetch_checksum(version: str, target: str) -> str:
    url = checksum_url(version, target)
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            contents = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        raise PackagingError(
            f"could not download {url}: {error.code} {error.reason}. "
            "Check that this release exists and published both architectures."
        ) from error
    except urllib.error.URLError as error:
        raise PackagingError(f"could not download {url}: {error.reason}") from error
    return parse_checksum(contents, archive_name(version, target))


def render(template: str, values: dict[str, str]) -> str:
    """Substitutes every `@KEY@` placeholder, refusing to leave any behind."""
    rendered = template
    for key, value in values.items():
        rendered = rendered.replace(f"@{key}@", value)
    remaining = sorted(set(re.findall(r"@([A-Z0-9_]+)@", rendered)))
    if remaining:
        raise PackagingError(f"template placeholders left unrendered: {remaining}")
    return rendered


def package_values(
    pkgname: str, version: str, pkgrel: int, checksums: dict[str, str]
) -> dict[str, str]:
    package = PACKAGES[pkgname]
    return {
        "PKGNAME": pkgname,
        "PKGVER": package_version(version),
        "PKGREL": str(pkgrel),
        "RELEASEVER": version,
        "PKGDESC": package["description"],
        "CHANNEL": package["channel"],
        "ALTERNATE": package["alternate"],
        "SHA256_X86_64": checksums["x86_64"],
        "SHA256_AARCH64": checksums["aarch64"],
    }


def write_srcinfo(directory: pathlib.Path) -> None:
    try:
        srcinfo = subprocess.run(
            ["makepkg", "--printsrcinfo"],
            cwd=directory,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    except FileNotFoundError as error:
        raise PackagingError(
            "makepkg is not available; re-run on Arch or pass --skip-srcinfo "
            "and regenerate .SRCINFO before publishing."
        ) from error
    except subprocess.CalledProcessError as error:
        raise PackagingError(f"makepkg --printsrcinfo failed: {error.stderr}") from error
    (directory / ".SRCINFO").write_text(srcinfo)


def update_package(
    root: pathlib.Path, pkgname: str, version: str, pkgrel: int, skip_srcinfo: bool
) -> None:
    template = (root / "packaging" / "aur" / "PKGBUILD.in").read_text()
    checksums = {target: fetch_checksum(version, target) for target in TARGETS}
    directory = root / "packaging" / "aur" / pkgname
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "PKGBUILD").write_text(
        render(template, package_values(pkgname, version, pkgrel, checksums))
    )
    if not skip_srcinfo:
        write_srcinfo(directory)
    print(f"{pkgname}: {package_version(version)}-{pkgrel} (release v{version})")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stable", help="stable release version, e.g. 0.7.0")
    parser.add_argument(
        "--rc",
        help="release-candidate version, e.g. 0.7.1-rc.2",
    )
    parser.add_argument("--pkgrel", type=int, default=1, help="package release number")
    parser.add_argument(
        "--skip-srcinfo",
        action="store_true",
        help="do not regenerate .SRCINFO (for machines without makepkg)",
    )
    arguments = parser.parse_args(argv)

    if arguments.stable is None and arguments.rc is None:
        parser.error("pass --stable, --rc, or both")

    root = pathlib.Path(__file__).resolve().parent.parent
    try:
        if arguments.stable is not None:
            update_package(
                root,
                "strata-bin",
                release_version(arguments.stable),
                arguments.pkgrel,
                arguments.skip_srcinfo,
            )
        if arguments.rc is not None:
            update_package(
                root,
                "strata-rc-bin",
                release_version(arguments.rc),
                arguments.pkgrel,
                arguments.skip_srcinfo,
            )
    except PackagingError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
