# SPDX-License-Identifier: MIT

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from qa_cost import durations, scores, with_cost, write


class ScoreTests(unittest.TestCase):
    def test_a_run_scores_its_median_over_the_median_of_all_medians(self):
        found = scores({
            "a/columns": [10.0, 30.0, 11.0],
            "b/columns": [5.0, 6.0, 4.0],
            "c/list": [2.0, 2.2],
            "d/icons": [0.01, 0.02, 0.01],
            "e/icons": [5.5],
        })
        self.assertEqual(found, {
            "a/columns": 2.2, "b/columns": 1.0, "c/list": 0.4, "d/icons": 0.1, "e/icons": 1.1,
        })

    def test_the_result_lines_give_each_runs_time_and_status(self):
        log = "\n".join([
            "running 4 tests",
            "RUN copy/columns",
            "test a_unit_test ... ok <0.001s>",
            "test copy/columns ... ok <12.345s>",
            "PASS copy/columns",
            "test copy/list-ru ... FAILED <3.5s>",
            "test copy/icons ... ignored, skipped: issue 9",
            "test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 12.35s",
        ])
        self.assertEqual(durations(log), {"copy/columns": (12.345, True), "copy/list-ru": (3.5, False)})


class HeaderTests(unittest.TestCase):
    HEADER = '{"scenario":"x","modes":["columns","list"],"note":"ü","nested":{"cost":1}}'

    def test_a_cost_is_added_last_and_nothing_else_changes(self):
        updated = with_cost(self.HEADER, {"columns": 1.5, "list": 0.1})
        self.assertEqual(updated, self.HEADER[:-1] + ',"cost":{"columns":1.5,"list":0.1}}')

    def test_only_the_cost_value_is_replaced_wherever_it_stands(self):
        header = '{ "scenario" : "x", "cost" : {"columns": 9.9} ,"modes":["columns"], "z":"}"}'
        updated = with_cost(header, {"columns": 0.4})
        self.assertEqual(
            updated, '{ "scenario" : "x", "cost" : {"columns":0.4} ,"modes":["columns"], "z":"}"}'
        )

    def test_a_scenario_is_timed_and_keeps_its_quality(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scenarios/hidden_files").mkdir(parents=True)
            path = root / "scenarios/hidden_files/alone.jsonl"
            header = ('{"scenario":"alone","modes":["columns","list"],"quality":{"impact":3,'
                      '"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"persists the toggle"}}')
            path.write_text(header + '\n{"key":"Escape"}\n')
            log = root / "qa.log"
            log.write_text("\n".join([
                "RUN alone/columns",
                "test alone/columns ... ok <1.000s>",
                "PASS alone/columns",
                "test alone/list ... ok <3.000s>",
            ]))
            with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(write([log], root / "scenarios"), 0)
            self.assertEqual(
                path.read_text(),
                header[:-1] + ',"cost":{"columns":0.5,"list":1.5}}\n{"key":"Escape"}\n',
            )

    def test_a_line_that_is_not_one_json_object_is_refused(self):
        for bad in ('["x"]', '{"a":1', '{"a" 1}', '{"a":1 "b":2}'):
            with self.assertRaises(ValueError, msg=bad):
                with_cost(bad, {"columns": 1.0})

    def test_write_changes_only_each_header_and_needs_a_time_for_every_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scenarios = root / "scenarios"
            (scenarios / "test_a").mkdir(parents=True)
            one = scenarios / "test_a/one.jsonl"
            two = scenarios / "test_a/two.jsonl"
            steps = '{"key":"Escape"}\n\n{"wait":{"panes":1}}'
            one.write_text('{"scenario":"one","modes":["columns","list"]}\n' + steps)
            two.write_text(
                '{"scenario":"two","modes":["columns"],"variants":[{"id":"us"},{"id":"ru"}],'
                '"cost":{"columns-us":7.0}}\n' + steps + "\n"
            )
            log = root / "qa.log"
            log.write_text("\n".join([
                "test one/columns ... ok <2.000s>",
                "test one/list ... ok <4.000s>",
                "test two/columns-us ... ok <4.000s>",
            ]))
            with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(write([log], scenarios), 1)
            self.assertNotIn("cost", one.read_text())

            with log.open("a") as output:
                output.write("\ntest two/columns-ru ... FAILED <8.000s>\n")
            with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(write([log, log], scenarios), 0)
            self.assertEqual(
                one.read_text(),
                '{"scenario":"one","modes":["columns","list"],"cost":{"columns":0.5,"list":1.0}}\n' + steps,
            )
            self.assertEqual(
                two.read_text(),
                '{"scenario":"two","modes":["columns"],"variants":[{"id":"us"},{"id":"ru"}],'
                '"cost":{"columns-us":1.0,"columns-ru":2.0}}\n' + steps + "\n",
            )
            self.assertEqual(json.loads(two.read_text().splitlines()[0])["scenario"], "two")


if __name__ == "__main__":
    unittest.main()
