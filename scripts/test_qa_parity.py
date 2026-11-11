# SPDX-License-Identifier: MIT

import contextlib
import io
import re
import tempfile
import tomllib
import unittest
from pathlib import Path

import qa_mutation
import qa_parity
import qa_parity_mutation
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


class ScenarioFilterTests(unittest.TestCase):
    def test_a_scenario_narrows_a_selection_to_its_own_runs(self):
        ledger = tomllib.loads(LEDGER)
        self.assertEqual(selected(ledger, "slice", "required", "escape-clears-selection"),
                         ["escape-clears-selection/columns"])
        self.assertEqual(selected(ledger, "g1", "required", "escape-clears-selection"), [])
        self.assertEqual(selected(ledger, "slice", "required", "rename-with-f2"), [])


class MutationTests(unittest.TestCase):
    def test_a_patch_is_named_for_its_scenario_and_needs_a_required_run_to_catch_it(self):
        ledger = tomllib.loads(LEDGER)
        with tempfile.TemporaryDirectory() as directory:
            for name in ("escape-clears-selection", "rename-with-f2"):
                (Path(directory) / f"{name}.patch").write_text("", encoding="utf-8")
            (Path(directory) / "notes.txt").write_text("", encoding="utf-8")
            found = qa_parity_mutation.patches(Path(directory))
        self.assertEqual(found, ["escape-clears-selection", "rename-with-f2"])
        # A pending run is not run, so it cannot catch anything.
        self.assertEqual(qa_parity_mutation.unguarded(found, ledger), ["rename-with-f2"])


class RepositoryTests(unittest.TestCase):
    def test_every_committed_patch_applies_to_strata_app_and_has_a_required_run(self):
        ledger = qa_parity.load()
        scenarios = qa_parity_mutation.patches()
        self.assertGreaterEqual(len(scenarios), 2)
        self.assertEqual(qa_parity_mutation.unguarded(scenarios, ledger), [])
        for scenario in scenarios:
            patch = qa_parity_mutation.MUTATIONS / f"{scenario}.patch"
            files = qa_mutation.patched_files(patch)
            self.assertTrue(files, patch)
            for file in files:
                self.assertTrue(file.startswith("crates/strata-app/"), file)

    def test_the_script_runs_the_mutations_alone_and_with_the_slice(self):
        script = (REPOSITORY / "scripts/qa-parity.sh").read_text(encoding="utf-8")
        self.assertEqual(script.count("qa_parity_mutation.py"), 3)

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
