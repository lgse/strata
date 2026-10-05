# SPDX-License-Identifier: GPL-3.0-or-later

import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import sign_update_manifest as signing

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests/fixtures/signed-update"


def test_key(directory, seed=7):
    path = directory / f"test-key-{seed}.pem"
    der = bytes.fromhex("302e020100300506032b657004220420") + bytes([seed]) * 32
    path.write_bytes(b"-----BEGIN PRIVATE KEY-----\n" + base64.b64encode(der) + b"\n-----END PRIVATE KEY-----\n")
    path.chmod(0o600)
    public = signing.openssl("pkey", "-in", path, "-pubout", "-outform", "DER")[12:]
    return path, public.hex()


class SignedUpdateTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.directory = Path(self.scratch.name)
        self.dist = self.directory / "dist"
        self.dist.mkdir()
        for target in signing.TARGETS:
            (self.dist / f"strata-0.12.0-{target}.tar.gz").write_bytes(b"strata update fixture\n")
        self.key, self.public = test_key(self.directory)

    def manifest(self):
        return signing.manifest_bytes(self.dist, "0.12.0", "a" * 40)

    def test_openssl_signature_matches_rust_interoperability_fixture(self):
        manifest = self.manifest()
        signatures = signing.sign_manifest(manifest, [self.key], [self.public])
        self.assertEqual(manifest, (FIXTURES / signing.MANIFEST_NAME).read_bytes())
        self.assertEqual(signatures, (FIXTURES / signing.SIGNATURES_NAME).read_bytes())
        self.assertEqual(self.public, (FIXTURES / "public-key.hex").read_text().strip())
        self.assertNotIn(self.public, json.loads((ROOT / "data/update-keys.json").read_text()))

    def test_manifest_binds_both_targets_and_exact_bytes(self):
        manifest = json.loads(self.manifest())
        self.assertEqual(manifest["repository"], "lgse/strata")
        self.assertEqual(manifest["source_commit"], "a" * 40)
        self.assertEqual({entry["target"] for entry in manifest["artifacts"]}, set(signing.TARGETS))
        for entry in manifest["artifacts"]:
            data = (self.dist / entry["name"]).read_bytes()
            self.assertEqual(entry["size"], len(data))
            self.assertEqual(entry["sha256"], hashlib.sha256(data).hexdigest())

    def test_missing_extra_empty_symlinked_and_oversized_archives_fail(self):
        path = next(self.dist.glob("*.tar.gz"))
        content = path.read_bytes()
        path.unlink()
        with self.assertRaises(ValueError):
            self.manifest()
        path.write_bytes(b"")
        with self.assertRaises(ValueError):
            self.manifest()
        path.unlink()
        path.symlink_to(self.key)
        with self.assertRaises(ValueError):
            self.manifest()
        path.unlink()
        path.write_bytes(content)
        with patch.object(signing, "MAX_ARCHIVE_BYTES", 1):
            with self.assertRaises(ValueError):
                self.manifest()
        (self.dist / "unexpected.tar.gz").write_bytes(b"extra")
        with self.assertRaises(ValueError):
            self.manifest()

    def test_rejects_malformed_release_identity(self):
        for version in ["../main", "01.2.3", "1.2.3+local", "1.2.3-rc.0", "1.2.3-rc.01", "1.2.18446744073709551616"]:
            with self.subTest(version=version), self.assertRaises(ValueError):
                signing.manifest_bytes(self.dist, version, "a" * 40)
        for commit in ["main", "a" * 39, "A" * 40, "../source"]:
            with self.subTest(commit=commit), self.assertRaises(ValueError):
                signing.manifest_bytes(self.dist, "0.12.0", commit)

    def test_dual_signing_requires_distinct_trusted_keys(self):
        other_key, other_public = test_key(self.directory, 8)
        signatures = json.loads(signing.sign_manifest(self.manifest(), [self.key, other_key], [self.public, other_public]))
        self.assertEqual(len(signatures["signatures"]), 2)
        for keys, trusted in [([self.key], []), ([self.key], [other_public]), ([self.key, self.key], [self.public])]:
            with self.assertRaises(ValueError):
                signing.sign_manifest(self.manifest(), keys, trusted)

    def test_cli_requires_key_and_does_not_publish_unsigned_metadata(self):
        environment = os.environ.copy()
        for name in ["UPDATE_SIGNING_KEY", "UPDATE_SIGNING_KEY_NEXT"]:
            environment.pop(name, None)
        command = ["python3", str(ROOT / "scripts/sign_update_manifest.py"), "--directory", str(self.dist), "--version", "0.12.0", "--source-commit", "a" * 40]
        result = subprocess.run(command, env=environment, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.dist / signing.MANIFEST_NAME).exists())
        trusted = self.directory / "keys.json"
        trusted.write_text(json.dumps([self.public]))
        environment["UPDATE_SIGNING_KEY"] = self.key.read_text()
        result = subprocess.run(command + ["--trusted-keys", str(trusted)], env=environment, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(self.key.read_bytes(), result.stdout + result.stderr)
        self.assertEqual((self.dist / signing.MANIFEST_NAME).read_bytes(), self.manifest())
        self.assertTrue((self.dist / signing.SIGNATURES_NAME).exists())
        repeated = subprocess.run(command + ["--trusted-keys", str(trusted)], env=environment, capture_output=True)
        self.assertNotEqual(repeated.returncode, 0)

    def test_signing_job_never_checks_out_the_selected_build_source(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        job = workflow.split("\n  sign:\n", 1)[1].split("\n  release:\n", 1)[0]
        self.assertIn("environment: release-signing", job)
        self.assertIn("ref: ${{ github.workflow_sha }}", job)
        self.assertNotIn("checkout_ref", job)
        self.assertNotIn("RELEASE_DEPLOY_KEY", job)
        self.assertIn("needs: [prepare, build, sign]", workflow)
        self.assertIn("UPDATE_SIGNING_KEY_NEXT", job)


if __name__ == "__main__":
    unittest.main()
