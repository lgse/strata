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


ALLOWANCES = """
[[flaky]]
scenario = "new-folder"
run = "columns"
reason = "since before the slice"
recorded = "notes.md"

[[stale_patch]]
patch = "cut"
reason = "since before the slice"
recorded = "notes.md"
"""

# The runner's own lines, tests/qa/harness.rs.
SUITE_LOG = """\
RUN new-folder/columns
test escape-clears-selection/columns ... ok <1.203s>
PASS escape-clears-selection/columns
test new-folder/columns ... FAILED <18.556s>
{more}
failures:

---- new-folder/columns stdout ----
new-folder/columns failed:

failures:
    new-folder/columns

test result: FAILED. 790 passed; {failed} failed; 2 ignored; 0 measured; 0 filtered out; finished in 66.60s
"""


def suite_log(*others, failed=None):
    more = "\n".join(f"test {other} ... FAILED <0.004s>" for other in others)
    return SUITE_LOG.format(more=more, failed=1 + len(others) if failed is None else failed)


class FlakyAllowanceTests(unittest.TestCase):
    def setUp(self):
        self.ledger = tomllib.loads(LEDGER + ALLOWANCES)

    def test_the_ledger_names_what_it_allows_and_a_ledger_without_any_allows_nothing(self):
        self.assertEqual(qa_parity.flaky(self.ledger), ["new-folder/columns"])
        self.assertEqual(qa_parity.stale_patches(self.ledger), ["cut"])
        bare = tomllib.loads(LEDGER)
        self.assertEqual((qa_parity.flaky(bare), qa_parity.stale_patches(bare)), ([], []))
        # The allowances are no runs of a gate.
        self.assertEqual(selected(self.ledger, "slice", "required"), selected(bare, "slice", "required"))

    def test_the_flaky_run_alone_is_passed_over_and_any_other_failure_stands(self):
        self.assertIsNone(qa_parity.refusal(self.ledger, suite_log()))
        self.assertEqual(qa_parity.excused(self.ledger, suite_log()), (["new-folder/columns"], []))
        for other in ("escape-clears-selection/list", "new-folder/list",
                      "the_parity_ledger_accounts_for_every_columns_run_of_the_slices_set"):
            with self.subTest(other):
                self.assertEqual(qa_parity.refusal(self.ledger, suite_log(other)),
                                 f"{other} failed in the whole GTK suite, and no allowance names them.")
        self.assertIn("new-folder/columns failed", qa_parity.refusal(tomllib.loads(LEDGER), suite_log()))

    def test_a_suite_that_failed_some_other_way_is_not_passed_over(self):
        for case, log, reason in (
            ("a build error names no test", "error: could not compile `strata`\n",
             "the whole GTK suite failed without naming a failed test."),
            ("the runner stopped before its result", suite_log().split("\ntest result")[0],
             "the whole GTK suite did not end with a count of the failures it named."),
            ("the result counts a failure no line names", suite_log(failed=2),
             "the whole GTK suite did not end with a count of the failures it named."),
        ):
            with self.subTest(case):
                self.assertEqual(qa_parity.refusal(self.ledger, log), reason)

    def run_command(self, log_text):
        with tempfile.TemporaryDirectory() as directory:
            ledger = Path(directory) / "slice.toml"
            ledger.write_text(LEDGER + ALLOWANCES, encoding="utf-8")
            log = Path(directory) / "suite.log"
            log.write_text(log_text, encoding="utf-8")
            printed, errors = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(printed), contextlib.redirect_stderr(errors):
                status = qa_parity.main(["slice", "--ledger", str(ledger), "--failed-in", str(log)])
        return status, printed.getvalue(), errors.getvalue()

    def test_the_command_prints_the_flaky_runs_or_refuses_the_whole_failure(self):
        self.assertEqual(self.run_command(suite_log()), (0, "new-folder/columns\n", ""))
        self.assertEqual(
            self.run_command(suite_log("escape-clears-selection/list")),
            (1, "", "NO ALLOWANCE: escape-clears-selection/list failed in the whole GTK suite, "
                    "and no allowance names them.\n"))
        status, printed, errors = self.run_command("error: could not compile `strata`\n")
        self.assertEqual((status, printed), (1, ""))
        self.assertTrue(errors.startswith("NO ALLOWANCE: "), errors)


class StalePatchTests(unittest.TestCase):
    def sort(self, stale, broken):
        patches = ["clipboard", "cut", "trash"]
        return qa_parity_mutation.sort_gtk_patches(patches, stale, lambda patch: patch not in broken)

    def test_a_stale_patch_is_left_out_only_while_the_ledger_names_it_and_it_stays_stale(self):
        self.assertEqual(self.sort(["cut"], ["cut"]), (["clipboard", "trash"], []))
        self.assertEqual(self.sort([], []), (["clipboard", "cut", "trash"], []))
        for case, stale, broken, problem in (
            ("a patch stops applying unnamed", ["cut"], ["cut", "trash"],
             "trash.patch does not apply, and the ledger does not name it stale"),
            ("a named patch applies again", ["cut"], [],
             "cut.patch applies again: remove its [[stale_patch]] entry from the ledger"),
            ("a named patch is no scenario's", ["paste"], [],
             "paste.patch is named stale, and no scenario names it as its mutation"),
        ):
            with self.subTest(case):
                self.assertEqual(self.sort(stale, broken)[1], [problem])


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

    def test_the_script_runs_each_targets_mutations_alone_and_with_the_slice(self):
        script = (REPOSITORY / "scripts/qa-parity.sh").read_text(encoding="utf-8")
        for target in ("rune", "gtk"):
            self.assertEqual(script.count(f'qa_parity_mutation.py" {target}'), 2, target)

    def test_the_gtk_patches_the_ledger_calls_stale_are_the_ones_that_do_not_apply(self):
        patches = [scenario.mutation for scenario in qa_mutation.chosen_scenarios([])]
        stale = qa_parity.stale_patches(qa_parity.load())
        chosen, problems = qa_parity_mutation.sort_gtk_patches(patches, stale)
        self.assertEqual(problems, [])
        self.assertEqual(sorted(chosen), sorted(set(patches) - set(stale)))
        self.assertTrue(chosen)

    def test_the_script_asks_the_ledger_which_failures_of_the_whole_suite_may_run_again(self):
        script = (REPOSITORY / "scripts/qa-parity.sh").read_text(encoding="utf-8")
        self.assertIn('qa_parity.py slice --failed-in "$output"', script)
        harness = (REPOSITORY / "tests/qa/harness.rs").read_text(encoding="utf-8")
        line = re.search(r'"(test \{\} \.\.\. FAILED[^"\\]*)', harness).group(1)
        self.assertEqual(qa_parity.failed_tests(line.replace("{}", "a/b").replace("{seconds:.3}", "1.000")),
                         ["a/b"])
        result = re.search(r'"\\n(test result: \{\}\. \{passed\} passed; \{\} failed;)', harness).group(1)
        self.assertEqual(qa_parity.counted_failures(
            result.replace("{}", "FAILED", 1).replace("{passed}", "7").replace("{}", "3")), 3)
        self.assertNotIn("--exact \"$1\"", script, "a flaky run is not run again")

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
