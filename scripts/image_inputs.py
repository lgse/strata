#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Input-addressed keys for the pinned test image and its dependency layers."""

import argparse
import hashlib
import json
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


IMAGE_INPUTS = (
    "tests/container/Dockerfile",
    "tests/container/install-packages.sh",
    "tests/container/gtk/coordinate-probe.py",
    "tests/container/gtk/gtk-4.22-accessible-bounds.patch",
)


def image_key(repository=REPOSITORY):
    return hashlib.sha256(
        "".join(digest(repository / path) for path in IMAGE_INPUTS).encode()
    ).hexdigest()


def dependency_key(repository=REPOSITORY):
    inputs = [image_key(repository)] + [digest(repository / name)
                                       for name in ("Cargo.toml", "Cargo.lock", ".dockerignore")]
    return hashlib.sha256("".join(inputs).encode()).hexdigest()


def source_key(repository=REPOSITORY):
    files = [repository / name for name in ("Cargo.toml", "Cargo.lock", "build.rs")]
    for directory in ("src", "data"):
        files.extend(path for path in (repository / directory).rglob("*") if path.is_file())
    inputs = [(str(path.relative_to(repository)), digest(path)) for path in sorted(files)]
    return hashlib.sha256(json.dumps(inputs).encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("image-key")
    sub.add_parser("dependency-key")
    args = parser.parse_args()
    print(image_key() if args.command == "image-key" else dependency_key())


if __name__ == "__main__":
    main()
