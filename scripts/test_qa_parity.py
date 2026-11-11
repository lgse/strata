# SPDX-License-Identifier: MIT

import contextlib
import io
import re
import tempfile
import tomllib
import unittest
from pathlib import Path

import qa_parity
from qa_parity import selected

REPOSITORY = Path(__file__).resolve().parents[1]

LEDGER = """
[[run]]
scenario = "arrow-keys-move-focus"
run = "columns-arrows"
gate = "G1"
status = "required"

[[run]]
scenario = "escape-clears-selection"
run = "columns"
gate = "G2"
status = "required"

[[run]]
scenario = "context-menu-keeps-selection"
run = "columns"
gate = "G2"
status = "pending"
reason = "component"
blocker = "the menu work package"

[[run]]
scenario = "rename-with-f2"
run = "columns-keyboard"
gate = "G3"
status = "pending"
reason = "harness"
blocker = "field"
"""


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.ledger = tomllib.loads(LEDGER)

    def test_a_gate_names_its_own_runs_with_the_status(self):
        self.assertEqual(selected(self.ledger, "g1", "required"), ["arrow-keys-move-focus/columns-arrows"])
        self.assertEqual(selected(self.ledger, "g2", "required"), ["escape-clears-selection/columns"])
        self.assertEqual(selected(self.ledger, "g2", "pending"), ["context-menu-keeps-selection/columns"])
        self.assertEqual(selected(self.ledger, "g3", "required"), [])

    def test_the_slice_is_every_gate_in_ledger_order(self):
        self.assertEqual(selected(self.ledger, "slice", "required"),
                         ["arrow-keys-move-focus/columns-arrows", "escape-clears-selection/columns"])
        self.assertEqual(selected(self.ledger, "slice", "pending"),
                         ["context-menu-keeps-selection/columns", "rename-with-f2/columns-keyboard"])

    def test_the_command_prints_one_run_a_line_from_the_ledger_file(self):
        with tempfile.TemporaryDirectory() as directory:
            ledger = Path(directory) / "slice.toml"
            ledger.write_text(LEDGER, encoding="utf-8")
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                self.assertEqual(qa_parity.main(["g2", "--status", "pending", "--ledger", str(ledger)]), 0)
        self.assertEqual(printed.getvalue(), "context-menu-keeps-selection/columns\n")


class RepositoryTests(unittest.TestCase):
    def test_the_committed_ledger_has_gate_g1_required(self):
        with qa_parity.LEDGER.open("rb") as stream:
            ledger = tomllib.load(stream)
        self.assertEqual(selected(ledger, "g1", "required"), ["arrow-keys-move-focus/columns-arrows"])

    def test_the_script_names_the_runners_variables_and_artifact_folders(self):
        script = (REPOSITORY / "scripts/qa-parity.sh").read_text(encoding="utf-8")
        target = (REPOSITORY / "tests/qa/target.rs").read_text(encoding="utf-8")
        for constant in ("TARGET_VARIABLE", "RUNE_BINARY_VARIABLE"):
            variable = re.search(rf'{constant}: &str = "([A-Z_]+)"', target).group(1)
            self.assertIn(variable, script)
        for folder in re.findall(r'=> "(target/qa-artifacts[a-z-]*)"', target):
            self.assertIn(folder, script)


if __name__ == "__main__":
    unittest.main()
