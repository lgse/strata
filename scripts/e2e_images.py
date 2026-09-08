#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Input-addressed public E2E images and digest-pinned registry cache discovery."""

import argparse
import json
import os
import re
import subprocess
import sys
import time

from e2e_bundle import dependency_key, image_key


def references(repository, image_inputs, dependencies, uid, gid):
    repository = repository.lower()
    if (not re.fullmatch(r"[a-z0-9_.-]+/[a-z0-9_.-]+", repository)
            or any(part in (".", "..") for part in repository.split("/"))):
        raise ValueError("invalid image repository")
    if uid < 0 or gid < 0 or not all(re.fullmatch(r"[0-9a-f]{64}", key)
                                    for key in (image_inputs, dependencies)):
        raise ValueError("images require SHA256 input identifiers and nonnegative account IDs")
    prefix = f"ghcr.io/{repository}-e2e"
    profile = f"linux-amd64-{uid}-{gid}"
    return {"runtime": f"{prefix}-runtime:{image_inputs}-{profile}",
            "build": f"{prefix}-build:{dependencies}-{profile}",
            "cache": f"{prefix}-build:cache-{dependencies}-{profile}",
            "environment_cache": f"{prefix}-build:environment-{image_inputs}-{profile}"}


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


def discover_bases(refs, image_inputs, dependencies):
    deadline = time.monotonic() + 20

    def lookup(reference, labels=None):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return None
        try:
            return resolve(reference, timeout=remaining, expected_labels=labels)
        except subprocess.TimeoutExpired:
            return None

    labels = {"org.strata.e2e.inputs": image_inputs}
    build = lookup(refs["build"], {**labels, "org.strata.e2e.dependencies": dependencies})
    runtime = lookup(refs["runtime"], labels)
    cache = None
    if not build or not runtime:
        cache = lookup(refs["cache"]) or lookup(refs["environment_cache"])
    return {"dependencies_image": build or "dependencies", "runtime_image": runtime or "runtime",
            "cache_from": f"type=registry,ref={cache}" if cache else ""}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("references")
    bases = sub.add_parser("bases")
    bases.add_argument("--require-published", action="store_true")
    cache = sub.add_parser("cache")
    cache.add_argument("reference")
    cache.add_argument("--environment")
    args = parser.parse_args()
    try:
        if args.command in ("references", "bases"):
            refs = references(os.environ["GITHUB_REPOSITORY"], image_key(),
                              dependency_key(), os.getuid(), os.getgid())
            outputs = refs if args.command == "references" else discover_bases(refs, image_key(), dependency_key())
            missing = args.command == "bases" and (outputs["dependencies_image"] == "dependencies"
                                                    or outputs["runtime_image"] == "runtime")
            if missing and args.require_published:
                raise ValueError("matching published bases are required but unavailable; publish them and verify anonymous access first")
            for key, value in outputs.items():
                print(f"{key}={value}")
            if missing:
                print("::notice::Some published E2E bases are unavailable. Missing stages will use "
                      "verified caches/source bootstrap. Publish the matching trusted main inputs "
                      "and make both GHCR packages public for anonymous fork access.", file=sys.stderr)
        else:
            try:
                pinned = resolve(args.reference)
                if pinned is None and args.environment:
                    pinned = resolve(args.environment)
            except subprocess.TimeoutExpired:
                pinned = None
            if pinned:
                print(f"cache_from=type=registry,ref={pinned}")
            else:
                print("cache_from=")
                print("::notice::Published E2E environment unavailable (not published, private, or "
                      "registry unreachable). Falling back to verified dependency/index caches "
                      "and source bootstrap. Publish the trusted main-branch images and make "
                      "both GHCR packages public for anonymous fork access.", file=sys.stderr)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"E2E images: {error}\n")


if __name__ == "__main__":
    main()
