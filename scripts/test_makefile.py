"""Development launcher tests without opening windows or modifying desktop services."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ChooserLauncherTests(unittest.TestCase):
    def run_target(self, *arguments: str, build_status: int = 0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "calls.jsonl"
            for name in ("cargo", "python3"):
                executable = root / name
                executable.write_text(
                    f"#!{sys.executable}\n"
                    "import json, os, sys\n"
                    "from pathlib import Path\n"
                    "name = Path(sys.argv[0]).name\n"
                    "with open(os.environ['CALL_LOG'], 'a') as log:\n"
                    "    log.write(json.dumps([name, sys.argv[1:], os.getenv('GTK_A11Y')]) + '\\n')\n"
                    "sys.exit(int(os.environ['BUILD_STATUS']) if name == 'cargo' else 0)\n",
                    encoding="utf-8",
                )
                executable.chmod(0o755)
            result = subprocess.run(
                ["make", "run-chooser-dev", *arguments],
                cwd=ROOT,
                env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}",
                     "CALL_LOG": str(log), "BUILD_STATUS": str(build_status)},
                capture_output=True, text=True, check=False,
            )
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            return result, calls

    def test_default_rebuilds_before_launching_an_isolated_save_chooser(self):
        result, calls = self.run_target()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[0][:2], ["cargo", ["build"]])
        self.assertEqual(calls[1], ["python3", ["scripts/portal-test.py", "save", "--binary",
                                              "target/debug/strata", "--choices"], "none"])

    def test_case_and_options_can_be_overridden(self):
        result, calls = self.run_target(
            "CHOOSER_CASE=multiple", "CHOOSER_ARGS=--view icons --group-by-type"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[1][1], ["scripts/portal-test.py", "multiple", "--binary",
                                      "target/debug/strata", "--view", "icons", "--group-by-type"])

    def test_a_failed_build_does_not_launch_a_stale_binary(self):
        result, calls = self.run_target(build_status=1)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0][:2], ["cargo", ["build"]])
