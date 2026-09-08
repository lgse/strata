#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Reuse a verified local E2E environment; building it is always explicit."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from e2e_bundle import REPOSITORY, dependency_key, image_key
from e2e_images import check_image_labels, references


def inspect(engine, reference):
    result = subprocess.run([engine, "image", "inspect", reference],
                            capture_output=True, text=True, check=False)
    if result.returncode:
        return None
    images = json.loads(result.stdout)
    if not isinstance(images, list) or len(images) != 1:
        raise ValueError("expected exactly one local base image")
    return images[0]


def verify_image(image, inputs):
    check_image_labels({"os": image.get("Os"), "architecture": image.get("Architecture"),
                        "config": image.get("Config")}, {"org.strata.e2e.inputs": inputs})
    if not any(value.startswith("RUSTUP_HOME=") for value in image["Config"].get("Env", [])):
        raise ValueError("the selected base is a runtime-only image, not a Rust build environment")
    identity = image.get("Id", "")
    if not re.fullmatch(r"(?:sha256:)?[0-9a-f]{64}", identity):
        raise ValueError("invalid local image identity")
    return identity


def ensure_base(engine, repository=REPOSITORY):
    inputs = image_key(repository)
    local = f"strata-e2e:base-{inputs}-{os.getuid()}-{os.getgid()}"
    image = inspect(engine, local)
    if image is not None:
        return verify_image(image, inputs)
    # Published environments use the CI account. The runner supplies its own
    # minimal passwd/group entries and executes with the invoking user's IDs.
    refs = references(os.environ.get("STRATA_E2E_IMAGE_REPOSITORY", "lgse/strata"),
                      inputs, dependency_key(repository), 1001, 1001)
    remote = refs["environment_image"]
    image = inspect(engine, remote)
    if image is None:
        print("Pulling the pinned E2E base once; no Ubuntu bootstrap will run.", file=sys.stderr)
        result = subprocess.run([engine, "pull", "--platform=linux/amd64", remote],
                                stdout=sys.stderr, check=False)
        if result.returncode:
            raise ValueError("published base unavailable. Run `python3 scripts/e2e_base.py build` "
                             "only when an explicit local environment build is intended; "
                             "otherwise publish the matching environment first")
        image = inspect(engine, remote)
    if image is None:
        raise ValueError("the pulled base could not be inspected")
    identity = verify_image(image, inputs)
    subprocess.run([engine, "tag", identity, local], stdout=sys.stderr, check=True)
    return identity


def build_base(engine, repository=REPOSITORY):
    inputs = image_key(repository)
    local = f"strata-e2e:base-{inputs}-{os.getuid()}-{os.getgid()}"
    subprocess.run([engine, "build", "--platform=linux/amd64", "--target", "toolchain",
                    "--tag", local, "--build-arg", f"E2E_UID={os.getuid()}",
                    "--build-arg", f"E2E_GID={os.getgid()}",
                    "--label", f"org.strata.e2e.inputs={inputs}",
                    "--file", str(repository / "tests/e2e/Dockerfile"), str(repository)],
                   stdout=sys.stderr, check=True)
    image = inspect(engine, local)
    if image is None:
        raise ValueError("the built base could not be inspected")
    return verify_image(image, inputs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("ensure", "build", "verify"))
    parser.add_argument("reference", nargs="?")
    parser.add_argument("--engine", default=os.environ.get("STRATA_CONTAINER_ENGINE")
                        or ("podman" if shutil.which("podman") else "docker"))
    args = parser.parse_args()
    try:
        if args.command == "verify":
            if not args.reference:
                raise ValueError("verify requires an image reference")
            image = inspect(args.engine, args.reference)
            if image is None:
                raise ValueError("the explicitly selected base is not present locally")
            identity = verify_image(image, image_key())
        else:
            identity = (build_base if args.command == "build" else ensure_base)(args.engine)
        print(identity)
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"E2E base: {error}\n")


if __name__ == "__main__":
    main()
