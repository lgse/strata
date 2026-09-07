#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile

DOMAIN = b"strata-update-manifest-v1\0"
MANIFEST_NAME = "strata-update-manifest.json"
SIGNATURES_NAME = "strata-update-manifest.signatures.json"
TARGETS = ("aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu")
MAX_ARCHIVE_BYTES = 128 * 1024 * 1024
PUBLIC_KEY_PREFIX = bytes.fromhex("302a300506032b6570032100")
NUMBER = r"(?:0|[1-9][0-9]*)"
VERSION = re.compile(rf"{NUMBER}\.{NUMBER}\.{NUMBER}(?:-(?:(?:alpha|beta|rc)\.[1-9][0-9]*|nightly\.[0-9]{{8}}(?:\.[1-9][0-9]*)?))?")


def manifest_bytes(directory, version, source_commit):
    if not VERSION.fullmatch(version) or any(int(number) > 2**64 - 1 for number in re.findall(r"[0-9]+", version)):
        raise ValueError("Invalid release version")
    if not re.fullmatch(r"[0-9a-f]{40}", source_commit):
        raise ValueError("Invalid source commit")
    expected = {f"strata-{version}-{target}.tar.gz" for target in TARGETS}
    if {path.name for path in directory.glob("*.tar.gz")} != expected:
        raise ValueError("Release archives must exactly match both supported targets")
    artifacts = []
    for target in TARGETS:
        name = f"strata-{version}-{target}.tar.gz"
        path = directory / name
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode) or not 0 < info.st_size <= MAX_ARCHIVE_BYTES:
            raise ValueError(f"Invalid release archive: {name}")
        with path.open("rb") as archive:
            checksum = hashlib.file_digest(archive, "sha256").hexdigest()
        artifacts.append({"name": name, "target": target, "size": info.st_size, "sha256": checksum})
    value = {"schema": 1, "repository": "lgse/strata", "tag": f"v{version}", "source_commit": source_commit, "artifacts": artifacts}
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")


def openssl(*arguments):
    result = subprocess.run(["openssl", *map(str, arguments)], capture_output=True, timeout=30)
    if result.returncode:
        raise ValueError("OpenSSL could not process the Ed25519 release key/signature")
    return result.stdout


def sign_manifest(manifest, private_keys, trusted_keys):
    if not 1 <= len(private_keys) <= 2:
        raise ValueError("Supply one signing key, or two during rotation")
    if not trusted_keys or any(not re.fullmatch(r"[0-9a-f]{64}", key) for key in trusted_keys):
        raise ValueError("Invalid trusted public keys")
    signatures = []
    with tempfile.TemporaryDirectory(prefix="strata-sign-") as staging:
        staging = Path(staging)
        message = staging / "message"
        message.write_bytes(DOMAIN + manifest)
        for private_key in private_keys:
            public = openssl("pkey", "-in", private_key, "-pubout", "-outform", "DER")
            if len(public) != 44 or not public.startswith(PUBLIC_KEY_PREFIX):
                raise ValueError("Release signing requires an Ed25519 key")
            raw_public = public[len(PUBLIC_KEY_PREFIX):]
            if raw_public.hex() not in trusted_keys:
                raise ValueError("The signing key is not trusted by this checkout's updater")
            key_id = hashlib.sha256(raw_public).hexdigest()
            if any(entry["key_id"] == key_id for entry in signatures):
                raise ValueError("Duplicate release signing key")
            signature = openssl("pkeyutl", "-sign", "-rawin", "-inkey", private_key, "-in", message)
            if len(signature) != 64:
                raise ValueError("Invalid Ed25519 signature length")
            public_path = staging / "public.der"
            signature_path = staging / "signature"
            public_path.write_bytes(public)
            signature_path.write_bytes(signature)
            openssl("pkeyutl", "-verify", "-rawin", "-pubin", "-keyform", "DER", "-inkey", public_path, "-sigfile", signature_path, "-in", message)
            signatures.append({"key_id": key_id, "signature": signature.hex()})
    return (json.dumps({"schema": 1, "signatures": signatures}, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--trusted-keys", type=Path, default=Path(__file__).resolve().parent.parent / "data/update-keys.json")
    args = parser.parse_args()
    # Remove key material before spawning OpenSSL; only private temporary files carry it.
    private_pems = [os.environ.pop(name, "") for name in ("UPDATE_SIGNING_KEY", "UPDATE_SIGNING_KEY_NEXT")]
    if not private_pems[0]:
        parser.error("UPDATE_SIGNING_KEY is required; unsigned publication is not supported")
    try:
        manifest = manifest_bytes(args.directory, args.version, args.source_commit)
        trusted_keys = json.loads(args.trusted_keys.read_text())
        with tempfile.TemporaryDirectory(prefix="strata-release-keys-") as staging:
            private_keys = []
            for index, pem in enumerate(private_pems):
                if pem:
                    path = Path(staging) / f"key-{index}.pem"
                    with open(path, "x", opener=lambda name, flags: os.open(name, flags, 0o600)) as file:
                        file.write(pem)
                    private_keys.append(path)
            signatures = sign_manifest(manifest, private_keys, trusted_keys)
        for name, value in ((MANIFEST_NAME, manifest), (SIGNATURES_NAME, signatures)):
            path = args.directory / name
            with path.open("xb") as output:
                output.write(value)
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"Release signing failed: {error}\n")


if __name__ == "__main__":
    main()
