# SPDX-License-Identifier: MIT

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "tests/e2e/install-packages.sh"
SNAPSHOT = "2026/09/11"
SERVER = f"Server = https://archive.archlinux.org/repos/{SNAPSHOT}/$repo/os/$arch\n"


class PinnedPackageTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "etc/pacman.d").mkdir(parents=True)
        (self.root / "etc/pacman.conf").write_text("[options]\nSigLevel = Required DatabaseOptional\n")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        pacman = self.bin / "pacman"
        pacman.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "root = Path(os.environ['STRATA_E2E_PACMAN_ROOT'])\n"
            "with (root / 'calls.jsonl').open('a') as stream:\n"
            "    stream.write(json.dumps(sys.argv[1:]) + '\\n')\n"
            "if sys.argv[1:3] == ['-Syy', '--noconfirm'] and os.environ.get('CREATE_INDEXES') == '1':\n"
            "    sync = root / 'var/lib/pacman/sync'\n"
            "    sync.mkdir(parents=True, exist_ok=True)\n"
            "    (sync / 'core.db').write_text('signed core index')\n"
            "    (sync / 'extra.db').write_text('signed extra index')\n"
            "raise SystemExit(int(os.environ.get('PACMAN_STATUS', '0')))\n"
        )
        pacman.chmod(0o755)

    def run_installer(self, *arguments, snapshot=SNAPSHOT, create_indexes=False, status=0):
        result = subprocess.run(
            ["sh", str(SCRIPT), *arguments],
            env={
                **os.environ,
                "PATH": f"{self.bin}:{os.environ.get('PATH', os.defpath)}",
                "STRATA_E2E_ARCH_SNAPSHOT": snapshot,
                "STRATA_E2E_PACMAN_ROOT": str(self.root),
                "CREATE_INDEXES": "1" if create_indexes else "0",
                "PACMAN_STATUS": str(status),
            },
            text=True,
            capture_output=True,
            check=False,
        )
        log = self.root / "calls.jsonl"
        calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        return result, calls

    def test_indexes_pin_https_archive_and_force_metadata_refresh(self):
        result, calls = self.run_installer("indexes", create_indexes=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [["-Syy", "--noconfirm"]])
        self.assertEqual((self.root / "etc/pacman.d/mirrorlist").read_text(), SERVER)

    def test_install_upgrades_the_snapshot_as_one_signed_transaction(self):
        self.run_installer("indexes", create_indexes=True)
        result, calls = self.run_installer("install", "gtk4", "python-gobject")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            calls[-1],
            ["-Su", "--noconfirm", "--needed", "gtk4", "python-gobject"],
        )

    def test_install_requires_the_exact_mirror_and_both_repository_indexes(self):
        mirror = self.root / "etc/pacman.d/mirrorlist"
        sync = self.root / "var/lib/pacman/sync"
        for setup in (
            lambda: None,
            lambda: (mirror.parent.mkdir(parents=True, exist_ok=True), mirror.write_text("Server = https://example.test\n")),
            lambda: (mirror.write_text(SERVER), sync.mkdir(parents=True, exist_ok=True), (sync / "core.db").write_text("core")),
        ):
            with self.subTest(setup=setup):
                setup()
                result, calls = self.run_installer("install", "gtk4")
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(calls and calls[-1][:1] == ["-Su"])
                if (sync / "core.db").exists():
                    (sync / "core.db").unlink()
                mirror.unlink(missing_ok=True)

    def test_invalid_snapshots_and_unsigned_configuration_fail_before_pacman(self):
        for snapshot in ("latest", "2026/09/11;command", "../09/11"):
            with self.subTest(snapshot=snapshot):
                result, calls = self.run_installer("indexes", snapshot=snapshot)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])
        (self.root / "etc/pacman.conf").write_text("SigLevel = Never\n")
        result, calls = self.run_installer("indexes")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])

    def test_package_and_index_failures_are_propagated_without_retry(self):
        result, calls = self.run_installer("indexes", status=73)
        self.assertEqual(result.returncode, 73)
        self.assertEqual(len(calls), 1)
        self.run_installer("indexes", create_indexes=True)
        result, calls = self.run_installer("install", "gtk4", status=74)
        self.assertEqual(result.returncode, 74)
        self.assertEqual(calls[-1][0], "-Su")

    def test_modes_reject_missing_or_unexpected_package_arguments(self):
        for arguments in ((), ("indexes", "gtk4"), ("install",), ("unknown",)):
            with self.subTest(arguments=arguments):
                result, _ = self.run_installer(*arguments)
                self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
