#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Input-addressed public test images, resolved to digest-pinned references."""

import argparse
import json
import os
import re
import subprocess
import time

from image_inputs import dependency_key, image_key


def references(repository, image_inputs, dependencies, uid, gid):
    repository = repository.lower()
    if (not re.fullmatch(r"[a-z0-9_.-]+/[a-z0-9_.-]+", repository)
            or any(part in (".", "..") for part in repository.split("/"))):
        raise ValueError("invalid image repository")
    if uid < 0 or gid < 0 or not all(re.fullmatch(r"[0-9a-f]{64}", key)
                                    for key in (image_inputs, dependencies)):
        raise ValueError("images require SHA256 input identifiers and nonnegative account IDs")
    package = f"ghcr.io/{repository}-test-image"
    profile = f"linux-amd64-{uid}-{gid}"
    return {"build": f"{package}:{dependencies}-{profile}",
            "cache": f"{package}:cache-{dependencies}-{profile}",
            "environment_cache": f"{package}:environment-{image_inputs}-{profile}",
            "environment_image": f"{package}:base-{image_inputs}-{profile}"}


def check_image_labels(document, expected):
    if not isinstance(document, dict):
        raise ValueError("invalid published image configuration")
    image = document.get("linux/amd64", document)
    if not isinstance(image, dict) or image.get("os") != "linux" or image.get("architecture") != "amd64":
        raise ValueError("published base does not provide the required linux/amd64 environment")
    config = image.get("config", {})
    labels = config.get("Labels", {}) if isinstance(config, dict) else {}
    if not isinstance(labels, dict) or any(labels.get(key) != value for key, value in expected.items()):
        raise ValueError("published base input labels do not match this checkout; refusing to substitute it")


def resolve(reference, timeout=20, expected_labels=None):
    deadline = time.monotonic() + timeout
    result = subprocess.run(
        ["docker", "buildx", "imagetools", "inspect", reference, "--format", "{{json .Manifest}}"],
        capture_output=True, text=True, timeout=timeout, check=False)
    if result.returncode:
        return None
    manifest = json.loads(result.stdout)
    digest = manifest.get("digest", "") if isinstance(manifest, dict) else ""
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
        raise ValueError("registry returned an invalid manifest digest")
    pinned = reference.rsplit(":", 1)[0] + "@" + digest
    if expected_labels is not None:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise subprocess.TimeoutExpired("inspect image configuration", timeout)
        config = subprocess.run(
            ["docker", "buildx", "imagetools", "inspect", pinned, "--format", "{{json .Image}}"],
            capture_output=True, text=True, timeout=remaining, check=False)
        if config.returncode:
            return None
        check_image_labels(json.loads(config.stdout), expected_labels)
    return pinned


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("references")
    sub.add_parser("environment")
    sub.add_parser("build")
    args = parser.parse_args()
    try:
        inputs, dependencies = image_key(), dependency_key()
        refs = references(os.environ["GITHUB_REPOSITORY"], inputs, dependencies, os.getuid(), os.getgid())
        if args.command == "references":
            for key, value in refs.items():
                print(f"{key}={value}")
        elif args.command == "environment":
            pinned = resolve(refs["environment_image"], expected_labels={"org.strata.test.inputs": inputs})
            if not pinned:
                raise ValueError("the published local-development base is unavailable anonymously")
            print(f"environment_image={pinned}")
        else:
            pinned = resolve(refs["build"], expected_labels={"org.strata.test.inputs": inputs,
                                                             "org.strata.test.dependencies": dependencies})
            if not pinned:
                raise ValueError("the published build environment is unavailable anonymously")
            print(f"build_image={pinned}")
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"Test image registry: {error}\n")


if __name__ == "__main__":
    main()
