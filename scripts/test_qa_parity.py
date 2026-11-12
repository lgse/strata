# SPDX-License-Identifier: MIT

import contextlib
import io
import re
import subprocess
import tempfile
import unittest
import unittest.mock
from pathlib import Path

import qa_mutation
import qa_parity
import qa_parity_mutation
from qa_parity import selected

REPOSITORY = Path(__file__).resolve().parents[1]

# The ledger of two folders. `keyboard_navigation` and `inline_renaming` are of the slice's
# set, so their Columns runs have a gate.
FOLDERS = {
    "keyboard_navigation": """
[[run]]
scenario = "arrow-keys-move-focus"
run = "columns-arrows"
package = "V7"
gate = "G1"
status = "required"
host = "offscreen"

[[run]]
scenario = "escape-clears-selection"
run = "columns"
package = "V8"
gate = "G2"
status = "required"
host = "offscreen"

[[run]]
scenario = "escape-clears-selection"
run = "icons"
package = "A1"
status = "required"
host = "offscreen"

[[run]]
scenario = "context-menu-keeps-selection"
run = "columns"
package = "A6"
gate = "G2"
status = "pending"
reason = "component"
blocker = "the menu work package"
""",
    "inline_renaming": """
[[run]]
scenario = "rename-with-f2"
run = "columns-keyboard"
package = "A7"
gate = "G3"
status = "pending"
reason = "harness"
blocker = "field"

[[run]]
scenario = "rename-with-f2"
run = "list-keyboard"
package = "A2"
status = "pending"
reason = "scope"
blocker = "A2"
""",
    "visual_baselines": """
[[run]]
scenario = "columns-view-baseline"
run = "columns"
package = "E1"
status = "excluded"
ruling = "the owner, 2026-11-11"
""",
}


@contextlib.contextmanager
def ledger_directory(allowances=""):
    """A ledger directory holding FOLDERS, and slice.toml when there are allowances."""

    with tempfile.TemporaryDirectory() as directory:
        for folder, text in FOLDERS.items():
            (Path(directory) / f"{folder}.toml").write_text(text, encoding="utf-8")
        if allowances:
            (Path(directory) / qa_parity.ALLOWANCES).write_text(allowances, encoding="utf-8")
        yield Path(directory)


def load(allowances=""):
    with ledger_directory(allowances) as directory:
        return qa_parity.load(directory)


def run_main(*arguments, allowances=""):
    """The command's exit status, output and errors over the ledger of FOLDERS."""

    printed, errors = io.StringIO(), io.StringIO()
    with ledger_directory(allowances) as directory:
        with contextlib.redirect_stdout(printed), contextlib.redirect_stderr(errors):
            status = qa_parity.main([*arguments, "--ledger", str(directory)])
    return status, printed.getvalue(), errors.getvalue()


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.ledger = load()

    def test_a_gate_names_its_own_runs_with_the_status(self):
        self.assertEqual(selected(self.ledger, "g1", "required"), ["arrow-keys-move-focus/columns-arrows"])
        self.assertEqual(selected(self.ledger, "g2", "required"), ["escape-clears-selection/columns"])
        self.assertEqual(selected(self.ledger, "g2", "pending"), ["context-menu-keeps-selection/columns"])
        self.assertEqual(selected(self.ledger, "g3", "required"), [])

    def test_the_slice_is_every_gate_whoever_owns_the_run_and_no_run_without_one(self):
        self.assertEqual(selected(self.ledger, "slice", "required"),
                         ["arrow-keys-move-focus/columns-arrows", "escape-clears-selection/columns"])
        self.assertEqual(selected(self.ledger, "slice", "pending"),
                         ["rename-with-f2/columns-keyboard", "context-menu-keeps-selection/columns"])

    def test_a_folder_a_package_and_all_name_their_runs_in_every_view(self):
        self.assertEqual(selected(self.ledger, "keyboard_navigation", "required"),
                         ["arrow-keys-move-focus/columns-arrows", "escape-clears-selection/columns",
                          "escape-clears-selection/icons"])
        self.assertEqual(selected(self.ledger, "inline_renaming", "pending"),
                         ["rename-with-f2/columns-keyboard", "rename-with-f2/list-keyboard"])
        self.assertEqual(selected(self.ledger, "A1", "required"), ["escape-clears-selection/icons"])
        self.assertEqual(selected(self.ledger, "V8", "required"), ["escape-clears-selection/columns"])
        self.assertEqual(selected(self.ledger, "A7", "pending"), ["rename-with-f2/columns-keyboard"])
        self.assertEqual(selected(self.ledger, "A7", "required"), [])
        self.assertEqual(len(selected(self.ledger, "all", "required")), 3)
        self.assertEqual(selected(self.ledger, "all", "excluded"), ["columns-view-baseline/columns"])

    def test_a_selection_the_ledger_does_not_have_is_refused_by_name(self):
        for unknown in ("A99", "tabs", "G1", "mutations"):
            with self.subTest(unknown):
                with self.assertRaises(qa_parity.UnknownSelection):
                    selected(self.ledger, unknown, "required")
                status, printed, errors = run_main(unknown)
                self.assertEqual((status, printed), (2, ""))
                self.assertIn(repr(unknown), errors)

    def test_the_command_prints_one_run_a_line_from_the_ledger_directory(self):
        self.assertEqual(run_main("g2", "--status", "pending"),
                         (0, "context-menu-keeps-selection/columns\n", ""))
        self.assertEqual(run_main("A2", "--status", "pending"), (0, "rename-with-f2/list-keyboard\n", ""))

    def test_a_required_run_on_the_session_host_is_refused_and_not_run_offscreen(self):
        session = FOLDERS["keyboard_navigation"].replace(
            'package = "A1"\nstatus = "required"\nhost = "offscreen"',
            'package = "A1"\nstatus = "required"\nhost = "session"')
        with unittest.mock.patch.dict(FOLDERS, {"keyboard_navigation": session}):
            self.assertEqual(qa_parity.needs_session(load(), "A1"), ["escape-clears-selection/icons"])
            status, printed, errors = run_main("keyboard_navigation")
            self.assertEqual((status, printed), (3, ""))
            self.assertIn("escape-clears-selection/icons", errors)
            # The gate does not name the run, and a survey runs no required run.
            self.assertEqual(run_main("g2")[0], 0)
            self.assertEqual(run_main("A1", "--status", "pending"), (0, "", ""))


class CountTests(unittest.TestCase):
    def test_a_selection_is_counted_by_status_reason_host_view_and_package(self):
        status, printed, errors = run_main("all", "--counts")
        self.assertEqual((status, errors), (0, ""))
        self.assertEqual(printed, """\
== all: 7 run(s) in 3 folder(s) ==
required      3
pending       3
  component     1
  harness       1
  scope         1
excluded      1
host      offscreen 3, session 0
view      columns 5, icons 1, list 1
package    runs  required   pending  excluded
A1            1         1         0         0
A2            1         0         1         0
A6            1         0         1         0
A7            1         0         1         0
E1            1         0         0         1
V7            1         1         0         0
V8            1         1         0         0
""")
        lines = run_main("inline_renaming", "--counts")[1].splitlines()
        self.assertEqual(lines[0], "== inline_renaming: 2 run(s) in 1 folder(s) ==")
        self.assertEqual(lines[1:3], ["required      0", "pending       2"])

    def test_packages_sort_by_track_and_then_by_number(self):
        ledger = {"run": [{"package": package} for package in ("V8", "A10", "E1", "A2", "H4")]}
        self.assertEqual(qa_parity.packages(ledger), ["A2", "A10", "E1", "H4", "V8"])


class ScenarioFilterTests(unittest.TestCase):
    def test_a_scenario_narrows_a_selection_to_its_own_runs(self):
        ledger = load()
        self.assertEqual(selected(ledger, "slice", "required", "escape-clears-selection"),
                         ["escape-clears-selection/columns"])
        self.assertEqual(selected(ledger, "all", "required", "escape-clears-selection"),
                         ["escape-clears-selection/columns", "escape-clears-selection/icons"])
        self.assertEqual(selected(ledger, "g1", "required", "escape-clears-selection"), [])
        self.assertEqual(selected(ledger, "slice", "required", "rename-with-f2"), [])


ALLOWANCES = """
[[flaky]]
scenario = "new-folder"
run = "columns"
reason = "since before the slice"
recorded = "notes.md"
issue = "lgse/strata#1"

[[stale_patch]]
patch = "cut"
reason = "since before the slice"
recorded = "notes.md"
issue = "lgse/strata#2"
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
        self.ledger = load(ALLOWANCES)

    def test_the_ledger_names_what_it_allows_and_a_ledger_without_any_allows_nothing(self):
        self.assertEqual(qa_parity.flaky(self.ledger), ["new-folder/columns"])
        self.assertEqual(qa_parity.stale_patches(self.ledger), ["cut"])
        bare = load()
        self.assertEqual((qa_parity.flaky(bare), qa_parity.stale_patches(bare)), ([], []))
        # The allowances are no runs of a selection, and their file is no folder's.
        self.assertEqual(self.ledger["run"], bare["run"])
        self.assertNotIn("slice", qa_parity.folders(self.ledger))

    def test_the_flaky_run_alone_is_passed_over_and_any_other_failure_stands(self):
        self.assertIsNone(qa_parity.refusal(self.ledger, suite_log()))
        self.assertEqual(qa_parity.excused(self.ledger, suite_log()), (["new-folder/columns"], []))
        for other in ("escape-clears-selection/list", "new-folder/list",
                      "the_parity_ledger_accounts_for_every_run_of_every_scenario_folder"):
            with self.subTest(other):
                self.assertEqual(qa_parity.refusal(self.ledger, suite_log(other)),
                                 f"{other} failed in the whole GTK suite, and no allowance names them.")
        self.assertIn("new-folder/columns failed", qa_parity.refusal(load(), suite_log()))

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
            log = Path(directory) / "suite.log"
            log.write_text(log_text, encoding="utf-8")
            return run_main("slice", "--failed-in", str(log), allowances=ALLOWANCES)

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
        ledger = load()
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
        self.assertEqual(selected(qa_parity.load(), "g1", "required"),
                         ["arrow-keys-move-focus/columns-arrows"])

    def test_the_committed_ledger_has_a_file_for_each_scenario_folder_and_no_other(self):
        ledger = qa_parity.load()
        scenarios = REPOSITORY / "tests/qa/scenarios"
        self.assertEqual(qa_parity.folders(ledger), sorted(path.name for path in scenarios.iterdir()))
        files = {path.stem for path in qa_parity.LEDGER.glob("*.toml")}
        self.assertEqual(files - set(qa_parity.folders(ledger)), {Path(qa_parity.ALLOWANCES).stem})
        self.assertEqual((qa_parity.flaky(ledger), qa_parity.stale_patches(ledger)),
                         (["new-folder-with-selection-groups/columns"], ["cut"]))

    def test_no_folder_or_package_takes_the_name_of_another_selection(self):
        ledger = qa_parity.load()
        reserved = {*qa_parity.GATES, qa_parity.EVERY, "mutations", "gtk-mutations"}
        names = [*qa_parity.folders(ledger), *qa_parity.packages(ledger)]
        self.assertEqual(reserved & set(names), set())
        self.assertEqual(len(names), len(set(names)))

    def test_every_required_run_of_the_committed_ledger_runs_on_the_offscreen_host(self):
        self.assertEqual(qa_parity.needs_session(qa_parity.load(), "all"), [])

    def test_the_script_takes_any_selection_and_counts_without_the_image(self):
        script = (REPOSITORY / "scripts/qa-parity.sh").read_text(encoding="utf-8")
        counted = script.index('qa_parity.py" "$selection" --counts')
        self.assertLess(counted, script.index("qa_container_prepare"))
        self.assertIn('qa_parity.py" "$selection" --status "$status"', script)
        command = [str(REPOSITORY / "scripts/qa-parity.sh")]
        refused = subprocess.run([*command, "no_such_folder"], capture_output=True, text=True, check=False)
        self.assertEqual((refused.returncode, refused.stdout), (2, ""))
        self.assertIn("'no_such_folder' is no selection", refused.stderr)
        counts = subprocess.run([*command, "slice", "--counts"], capture_output=True, text=True, check=True)
        self.assertEqual(counts.stdout, qa_parity.counts(qa_parity.load(), "slice") + "\n")

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
