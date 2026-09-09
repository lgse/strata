# SPDX-License-Identifier: MIT

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "tests/e2e/install-packages.sh"
SNAPSHOT = "http://snapshot.ubuntu.com/ubuntu/20260901T000000Z"
PREFIX = "snapshot.ubuntu.com_ubuntu_20260901T000000Z"
MIRROR_PREFIX = "archive.ubuntu.com_ubuntu"


class PinnedPackageTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.sources = self.root / "etc/apt/sources.list"
        self.lists = self.root / "var/lib/apt/lists"
        self.lists.mkdir(parents=True)
        self.sources.parent.mkdir(parents=True)
        self.original = f"# pinned profile\ndeb [check-valid-until=no] {SNAPSHOT} noble main universe\n"
        self.sources.write_text(self.original)
        self.metadata = {"_dists_noble_InRelease": "signed release fixture",
                         "_dists_noble_main_binary-amd64_Packages.lz4": "pinned versions and hashes"}
        for suffix, contents in self.metadata.items():
            (self.lists / (PREFIX + suffix)).write_text(contents)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        apt = self.bin / "apt-get"
        apt.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "root = Path(os.environ['STRATA_E2E_APT_ROOT'])\n"
            "call = {'args': sys.argv[1:], 'sources': (root/'etc/apt/sources.list').read_text(),\n"
            "        'indexes': {p.name: p.read_text() for p in (root/'var/lib/apt/lists').iterdir()}}\n"
            "with (root/'calls.jsonl').open('a') as f: f.write(json.dumps(call)+'\\n')\n"
            "if '--print-uris' in sys.argv:\n"
            "    uris = root/'uris.txt'\n"
            "    print(uris.read_text() if uris.exists() else '')\n"
            "    raise SystemExit(0)\n"
            "key = 'DOWNLOAD_STATUS' if '--download-only' in sys.argv else 'INSTALL_STATUS'\n"
            "raise SystemExit(int(os.environ[key]))\n"
        )
        apt.chmod(0o755)
        helper = self.root / "usr/lib/apt/apt-helper"
        helper.parent.mkdir(parents=True)
        helper.write_text(
            f"#!{sys.executable}\n"
            "import hashlib, json, os, sys\n"
            "from pathlib import Path\n"
            "root = Path(os.environ['STRATA_E2E_APT_ROOT'])\n"
            "with (root/'helper.jsonl').open('a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')\n"
            "data = os.environ['ARCHIVE_PAYLOAD'].encode()\n"
            "if sys.argv[-1] != 'SHA256:'+hashlib.sha256(data).hexdigest(): raise SystemExit(100)\n"
            "Path(sys.argv[-2]).write_bytes(data)\n"
        )
        helper.chmod(0o755)

    def run_installer(self, download=0, install=0, payload="pinned archive fixture"):
        result = subprocess.run(
            ["sh", str(SCRIPT), "libgtk-4-dev", "fonts-cantarell"],
            env={**os.environ, "PATH": f"{self.bin}:{os.environ.get('PATH', os.defpath)}",
                 "STRATA_E2E_APT_ROOT": str(self.root), "STRATA_E2E_SNAPSHOT_URL": SNAPSHOT,
                 "DOWNLOAD_STATUS": str(download), "INSTALL_STATUS": str(install), "ARCHIVE_PAYLOAD": payload},
            text=True, capture_output=True, check=False,
        )
        log = self.root / "calls.jsonl"
        calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        return result, calls

    def assert_restored(self):
        self.assertEqual(self.sources.read_text(), self.original)
        self.assertFalse(list(self.sources.parent.glob("sources.list.strata-*")))
        self.assertFalse(list(self.lists.glob(MIRROR_PREFIX + "_*")))
        for suffix, contents in self.metadata.items():
            self.assertEqual((self.lists / (PREFIX + suffix)).read_text(), contents)

    def test_mirror_only_downloads_using_unchanged_signed_snapshot_indexes(self):
        result, calls = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 2)
        download, install = calls
        self.assertEqual(download["args"], ["--download-only", "install", "--yes",
                                           "--no-install-recommends", "libgtk-4-dev", "fonts-cantarell"])
        self.assertEqual(download["sources"], self.original.replace(SNAPSHOT, "https://archive.ubuntu.com/ubuntu"))
        for suffix, contents in self.metadata.items():
            self.assertEqual(download["indexes"][MIRROR_PREFIX + suffix], contents)
        self.assertEqual(install["sources"], self.original)
        self.assertEqual(install["args"], download["args"][1:])
        self.assert_restored()

    def test_unavailable_mirror_versions_fall_back_without_changing_resolution(self):
        result, calls = self.run_installer(download=100)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 3)
        self.assertIn("--print-uris", calls[1]["args"])
        self.assertIn("Acquire::ForceHash=SHA256", calls[1]["args"])
        self.assertEqual(calls[2]["sources"], self.original)
        self.assertIn("Launchpad archive fallback", result.stderr)
        self.assert_restored()

    def archive_uri(self, checksum=None, cached="fixture_1_amd64.deb"):
        checksum = checksum or hashlib.sha256(b"pinned archive fixture").hexdigest()
        (self.root / "uris.txt").write_text(
            "'https://archive.ubuntu.com/ubuntu/pool/main/f/fixture/fixture_1_amd64.deb' "
            f"{cached} 22 SHA256:{checksum}\n")
        return checksum

    def test_archival_fallback_uses_the_signed_snapshot_hash_and_original_install_sources(self):
        checksum = self.archive_uri()
        result, calls = self.run_installer(download=100)
        self.assertEqual(result.returncode, 0, result.stderr)
        helper = json.loads((self.root / "helper.jsonl").read_text())
        self.assertEqual(helper[-3], "https://launchpad.net/ubuntu/+archive/primary/+files/fixture_1_amd64.deb")
        self.assertEqual(helper[-1], "SHA256:" + checksum)
        self.assertEqual(calls[-1]["sources"], self.original)
        self.assertEqual(sum("--download-only" not in call["args"] for call in calls), 1)
        self.assert_restored()

    def test_bad_archival_bytes_fall_back_without_becoming_an_installable_cache_entry(self):
        self.archive_uri()
        result, calls = self.run_installer(download=100, install=87, payload="different bytes")
        self.assertEqual(result.returncode, 87)
        self.assertFalse((self.root / "var/cache/apt/archives/fixture_1_amd64.deb").exists())
        self.assertIn("completing downloads from the snapshot", result.stderr)
        self.assertEqual(sum("--download-only" not in call["args"] for call in calls), 1)
        self.assert_restored()

    def test_valid_local_archives_do_not_contact_the_archival_mirror(self):
        self.archive_uri()
        cached = self.root / "var/cache/apt/archives/fixture_1_amd64.deb"
        cached.parent.mkdir(parents=True)
        cached.write_bytes(b"pinned archive fixture")
        result, _ = self.run_installer(download=100)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "helper.jsonl").exists())
        self.assert_restored()

    def test_untrusted_archive_destinations_or_missing_sha256_are_rejected(self):
        for checksum, cached in (("bad", "fixture.deb"), ("a" * 64, "../outside.deb")):
            with self.subTest(checksum=checksum, cached=cached):
                self.archive_uri(checksum, cached)
                result, _ = self.run_installer(download=100)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.root / "helper.jsonl").exists())
                self.assert_restored()

    def test_installation_failure_is_propagated_and_not_retried(self):
        result, calls = self.run_installer(install=87)
        self.assertEqual(result.returncode, 87, result.stderr)
        self.assertEqual(len(calls), 2)
        self.assert_restored()

    def test_missing_release_or_package_indexes_fail_before_contacting_a_mirror(self):
        for suffix in self.metadata:
            with self.subTest(missing=suffix):
                path = self.lists / (PREFIX + suffix)
                contents = path.read_text()
                path.unlink()
                result, calls = self.run_installer()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])
                self.assertEqual(self.sources.read_text(), self.original)
                path.write_text(contents)

    def test_existing_mirror_indexes_are_never_overwritten_or_used(self):
        path = self.lists / (MIRROR_PREFIX + "_dists_noble_InRelease")
        path.write_text("a different repository state")
        result, calls = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])
        self.assertEqual(path.read_text(), "a different repository state")
        self.assertEqual(self.sources.read_text(), self.original)


if __name__ == "__main__":
    unittest.main()
