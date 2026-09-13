#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Create static, checksum-bound two-executable release metadata (never run artifacts)."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inspect_elf(path, target):
    size = path.stat().st_size
    with path.open("rb") as stream:
        header = stream.read(64)
        if len(header) != 64 or header[:7] != b"\x7fELF\x02\x01\x01":
            raise ValueError(f"Invalid ELF: {path.name}")
        kind, machine, version = struct.unpack_from("<HHI", header, 16)
        expected = {"x86_64-unknown-linux-gnu": 62, "aarch64-unknown-linux-gnu": 183}[target]
        offset = struct.unpack_from("<Q", header, 32)[0]
        entry_size, count = struct.unpack_from("<HH", header, 54)
        if kind not in (2, 3) or machine != expected or version != 1 or entry_size != 56 or not 0 < count <= 1024 or offset + entry_size * count > size:
            raise ValueError(f"Wrong architecture or malformed ELF: {path.name}")


def create(directory, release_tag, target, commit):
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-(?:alpha|beta|rc|nightly)\.[0-9.]+)?", release_tag):
        raise ValueError("Invalid release tag")
    if not re.fullmatch(r"[a-f0-9]{40}", commit):
        raise ValueError("Invalid source commit")
    for name in ("strata", "strata-media-helper"):
        path = directory / name
        if path.is_symlink() or not path.is_file() or path.stat().st_mode & 0o111 == 0:
            raise ValueError(f"Missing executable: {name}")
        inspect_elf(path, target)
    files = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError("Release bundles cannot contain symlinks")
        if path.is_file() and path.name != "bundle.json":
            files[path.relative_to(directory).as_posix()] = digest(path)
    manifest = dict(format=1, release_tag=release_tag, target=target,
                    source_commit=commit, media_protocol=1, files=files)
    (directory / "bundle.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--target", choices=("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"), required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    create(args.directory, args.release_tag, args.target, args.commit)


if __name__ == "__main__":
    main()
