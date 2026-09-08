#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Bind a CI binary and collected plan to the tested revision and image inputs."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess

REPOSITORY = Path(__file__).resolve().parents[1]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def image_key(repository=REPOSITORY):
    return hashlib.sha256("".join(digest(repository / path) for path in (
        "tests/e2e/Dockerfile", "tests/e2e/requirements.txt",
    )).encode()).hexdigest()


def source_key(repository=REPOSITORY):
    files = [repository / name for name in ("Cargo.toml", "Cargo.lock", "build.rs")]
    for directory in ("src", "data"):
        files.extend(path for path in (repository / directory).rglob("*") if path.is_file())
    inputs = [(str(path.relative_to(repository)), digest(path)) for path in sorted(files)]
    return hashlib.sha256(json.dumps(inputs).encode()).hexdigest()


def create(bundle, commit, repository=REPOSITORY):
    if not commit:
        raise ValueError("a bundle must identify the tested commit")
    metadata = {"schema": 1, "commit": commit, "image_key": image_key(repository),
                "source_key": source_key(repository),
                "files": {name: digest(bundle / name) for name in ("strata", "plan.json")}}
    (bundle / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")


def verify(bundle, commit, repository=REPOSITORY):
    metadata = json.loads((bundle / "metadata.json").read_text())
    if metadata.get("schema") != 1 or metadata.get("commit") != commit:
        raise ValueError("bundle is not from the tested revision")
    if metadata.get("image_key") != image_key(repository):
        raise ValueError("bundle rendering/toolchain inputs differ from the checkout")
    if metadata.get("source_key") != source_key(repository):
        raise ValueError("bundle application source differs from the checkout (including local edits)")
    for name in ("strata", "plan.json"):
        if digest(bundle / name) != metadata["files"][name]:
            raise ValueError(f"bundle checksum mismatch: {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("image-key")
    for command in ("create", "verify"):
        child = sub.add_parser(command)
        child.add_argument("bundle", type=Path)
        child.add_argument("--commit")
    args = parser.parse_args()
    if args.command == "image-key":
        print(image_key())
        return
    commit = args.commit or subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=REPOSITORY, text=True).strip()
    try:
        (create if args.command == "create" else verify)(args.bundle, commit)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"E2E bundle: {error}\n")


if __name__ == "__main__":
    main()
