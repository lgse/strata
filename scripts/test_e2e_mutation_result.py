# SPDX-License-Identifier: GPL-3.0-or-later

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from e2e_mutation_result import detected


class MutationRunnerTests(unittest.TestCase):
    def test_parallel_mutation_runs_finish_without_fail_fast_interruption(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root / "scripts"
            scripts.mkdir()
            for name in ("e2e-mutation-check.sh", "e2e_mutation_result.py"):
                shutil.copyfile(Path(__file__).with_name(name), scripts / name)
            mutations = root / "tests/e2e/mutations"
            mutations.mkdir(parents=True)
            (mutations / "click-modes.patch").touch()
            tools = root / "bin"
            tools.mkdir()
            git = tools / "git"
            git.write_text("#!/bin/sh\nexit 0\n")
            git.chmod(0o755)
            runner = scripts / "e2e.sh"
            runner.write_text(
                f"#!{sys.executable}\nimport sys\nfrom pathlib import Path\n"
                "for arg in sys.argv[1:]:\n"
                "    if arg.startswith('--junitxml='):\n"
                "        Path(arg.split('=', 1)[1]).write_text('<testsuite><testcase><failure/></testcase></testsuite>')\n"
                "        sys.exit(1 if '--maxfail=0' in sys.argv and '-x' not in sys.argv else 2)\n"
            )
            runner.chmod(0o755)
            environment = {**os.environ, "PATH": f"{tools}:{os.environ.get('PATH', os.defpath)}"}
            environment.pop("STRATA_BINARY", None)
            result = subprocess.run(["bash", str(scripts / "e2e-mutation-check.sh"), "click-modes"],
                                    env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("every mutation was detected", result.stdout)


class MutationResultTests(unittest.TestCase):
    def test_only_scenario_failures_count_as_detection(self):
        cases = [
            (1, '<failure message="assert False"/>', True),
            (1, '<error message="startup failed"/>', False),
            (1, '<failure/><error/>', False),
            (0, '', False),
            (2, '<failure/>', False),
            (5, '', False),
            (137, '<failure/>', False),
        ]
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.xml"
            for code, outcome, expected in cases:
                with self.subTest(code=code, outcome=outcome):
                    report.write_text(
                        f'<testsuites><testsuite><testcase>{outcome}'
                        '</testcase></testsuite></testsuites>'
                    )
                    self.assertEqual(detected(report, code), expected)

    def test_missing_or_incomplete_report_is_not_detection(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.xml"
            self.assertFalse(detected(report, 1))
            report.write_text('<testsuites>')
            self.assertFalse(detected(report, 1))
