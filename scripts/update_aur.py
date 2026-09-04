#!/usr/bin/env python3
"""Render Strata's AUR packages from `packaging/aur/PKGBUILD.in`."""

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
    r"^(?P<core>\d+\.\d+\.\d+)(?:-(?P<prerelease>(?:alpha|beta|rc)\.\d+|nightly\.\d{8}(?:\.\d+)?))?$"
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
        "description": "A fast, keyboard-first file manager for Linux (preview channel)",
    },
}


class PackagingError(ValueError):
    pass


def package_version(release_version: str) -> str:
    """Remove only the prerelease hyphen, preserving `vercmp` ordering."""
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


def stable_release_version(version: str) -> str:
    normalized = release_version(version)
    if "-" in normalized:
        raise PackagingError("--stable requires a final release version")
    return normalized


def preview_release_version(version: str) -> str:
    normalized = release_version(version)
    if "-nightly." in normalized:
        raise PackagingError("nightly releases are not packaged")
    return normalized


def archive_name(version: str, target: str) -> str:
    return f"strata-{version}-{target}-unknown-linux-gnu.tar.gz"


def checksum_url(version: str, target: str) -> str:
    return (
        f"{REPOSITORY_URL}/releases/download/v{version}/{archive_name(version, target)}.sha256"
    )


def parse_checksum(contents: str, expected_archive: str) -> str:
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


def generated_srcinfo(directory: pathlib.Path) -> str:
    try:
        return subprocess.run(
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


def write_srcinfo(directory: pathlib.Path) -> None:
    (directory / ".SRCINFO").write_text(generated_srcinfo(directory))


def update_package(
    root: pathlib.Path,
    pkgname: str,
    version: str,
    pkgrel: int,
    skip_srcinfo: bool,
    check: bool,
) -> None:
    template = (root / "packaging" / "aur" / "PKGBUILD.in").read_text()
    checksums = {target: fetch_checksum(version, target) for target in TARGETS}
    directory = root / "packaging" / "aur" / pkgname
    directory.mkdir(parents=True, exist_ok=True)
    pkgbuild = directory / "PKGBUILD"
    rendered = render(template, package_values(pkgname, version, pkgrel, checksums))
    if check:
        if not pkgbuild.is_file() or pkgbuild.read_text() != rendered:
            raise PackagingError(f"{pkgbuild} is not generated from PKGBUILD.in")
        if not skip_srcinfo:
            srcinfo = directory / ".SRCINFO"
            if not srcinfo.is_file() or srcinfo.read_text() != generated_srcinfo(directory):
                raise PackagingError(f"{srcinfo} is not generated from PKGBUILD")
    else:
        pkgbuild.write_text(rendered)
        if not skip_srcinfo:
            write_srcinfo(directory)
    print(f"{pkgname}: {package_version(version)}-{pkgrel} (release v{version})")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stable", help="stable release version, e.g. 0.9.0")
    parser.add_argument(
        "--preview",
        "--rc",
        dest="preview",
        help="newest non-nightly Preview-channel version, e.g. 0.10.0-rc.1 or 0.9.0",
    )
    parser.add_argument("--pkgrel", type=int, default=1, help="package release number")
    parser.add_argument(
        "--skip-srcinfo",
        action="store_true",
        help="do not regenerate .SRCINFO (for machines without makepkg)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify generated files without changing them",
    )
    arguments = parser.parse_args(argv)

    if arguments.stable is None and arguments.preview is None:
        parser.error("pass --stable, --preview, or both")
    if arguments.pkgrel < 1:
        parser.error("--pkgrel must be at least 1")

    root = pathlib.Path(__file__).resolve().parent.parent
    try:
        if arguments.stable is not None:
            update_package(
                root,
                "strata-bin",
                stable_release_version(arguments.stable),
                arguments.pkgrel,
                arguments.skip_srcinfo,
                arguments.check,
            )
        if arguments.preview is not None:
            update_package(
                root,
                "strata-rc-bin",
                preview_release_version(arguments.preview),
                arguments.pkgrel,
                arguments.skip_srcinfo,
                arguments.check,
            )
    except PackagingError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
