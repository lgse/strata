# SPDX-License-Identifier: GPL-3.0-or-later

import tempfile
import unittest
from pathlib import Path

from e2e_mutation_result import detected


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
