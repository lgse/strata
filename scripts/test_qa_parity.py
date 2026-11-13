# SPDX-License-Identifier: MIT

import contextlib
import io
import json
import re
import subprocess
import tempfile
import tomllib
import unittest
import unittest.mock
from pathlib import Path

import qa_mutation
import qa_parity
import qa_parity_check
import qa_parity_mutation
from qa_parity import selected

REPOSITORY = Path(__file__).resolve().parents[1]

# A corpus of three folders: each scenario's header, as tests/qa/scenarios holds them, and
# under `steps` its step lines when they are not the one every other file has.
# `keyboard_navigation` and `inline_renaming` are of the slice's set, so their Columns runs
# have a gate.
SCENARIOS = {
    "keyboard_navigation/arrow-keys-move-focus": {"modes": ["columns"], "variants": [{"id": "arrows"}]},
    "keyboard_navigation/context-menu-keeps-selection": {"modes": ["columns"]},
    "keyboard_navigation/escape-clears-selection": {"modes": ["columns", "icons"]},
    "inline_renaming/rename-with-f2": {"modes": ["columns", "list"], "variants": [{"id": "keyboard"}]},
    "visual_baselines/columns-view-baseline": {"modes": ["columns"]},
}

# Its ledger, each file in the runner's order.
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
scenario = "context-menu-keeps-selection"
run = "columns"
package = "A6"
gate = "G2"
status = "pending"
reason = "component"
blocker = "the menu work package"

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


# The scenarios the corpus has a Rune mutation patch for.
PATCHES = []

@contextlib.contextmanager
def corpus(allowances=""):
    """A directory holding SCENARIOS under `scenarios` and FOLDERS under `ledger`, with
    slice.toml when there are allowances and a patch for each of PATCHES."""

    with tempfile.TemporaryDirectory() as directory:
        ledger, scenarios = Path(directory) / "ledger", Path(directory) / "scenarios"
        (ledger / qa_parity.MUTATIONS).mkdir(parents=True)
        for patch in PATCHES:
            (ledger / qa_parity.MUTATIONS / f"{patch}.patch").write_text("", encoding="utf-8")
        for path, header in SCENARIOS.items():
            file = scenarios / f"{path}.jsonl"
            file.parent.mkdir(parents=True, exist_ok=True)
            header = {"scenario": file.stem, **header}
            steps = header.pop("steps", ['{"key":"Escape"}'])
            file.write_text("\n".join([json.dumps(header), *steps]) + "\n", encoding="utf-8")
        for folder, text in FOLDERS.items():
            (ledger / f"{folder}.toml").write_text(text, encoding="utf-8")
        if allowances:
            (ledger / qa_parity.ALLOWANCES).write_text(allowances, encoding="utf-8")
        yield ledger, scenarios


def load(allowances=""):
    with corpus(allowances) as (ledger, scenarios):
        return qa_parity.load(ledger, scenarios)


def run_main(*arguments, allowances=""):
    """The command's exit status, output and errors over the ledger of FOLDERS."""

    printed, errors = io.StringIO(), io.StringIO()
    with corpus(allowances) as (ledger, scenarios):
        with contextlib.redirect_stdout(printed), contextlib.redirect_stderr(errors):
            status = qa_parity.main([*arguments, "--ledger", str(ledger), "--scenarios", str(scenarios)])
    return status, printed.getvalue(), errors.getvalue()


def edited(folder, find, put):
    """FOLDERS with one file changed: `find`, which occurs once in it, becomes `put`."""

    assert FOLDERS[folder].count(find) == 1, find
    return unittest.mock.patch.dict(FOLDERS, {folder: FOLDERS[folder].replace(find, put)})


def edit(test, case, files):
    """A table's texts after a case's edit: `find` and `put`, `append`, `text` or `remove`."""

    files = dict(files)
    before = files.pop(case["file"], "")
    if case.get("remove"):
        return files
    if "find" in case:
        test.assertEqual(before.count(case["find"]), 1, case["name"])
    after = before.replace(case["find"], case.get("put", "")) if "find" in case else before + case.get("append", "")
    files[case["file"]] = case.get("text", after)
    return files


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

    def test_a_ledger_file_that_does_not_parse_is_refused_with_the_parsers_reason(self):
        # A stray `\'` in a blocker, as keyboard_navigation.toml once had (A2, #2234).
        with edited("keyboard_navigation", 'blocker = "the menu work package"',
                    'blocker = "the menu work package\\\' "'):
            for arguments in (("all", "--counts"), ("A2", "--status", "pending"), ("g2",)):
                with self.subTest(arguments):
                    status, printed, errors = run_main(*arguments)
                    self.assertEqual((status, printed), (2, ""))
                    self.assertIn("keyboard_navigation.toml: Unescaped '\\' in a string", errors)
                    self.assertNotIn("Traceback", errors)

    def test_the_command_prints_one_run_a_line_from_the_ledger_directory(self):
        self.assertEqual(run_main("g2", "--status", "pending"),
                         (0, "context-menu-keeps-selection/columns\n", ""))
        self.assertEqual(run_main("A2", "--status", "pending"), (0, "rename-with-f2/list-keyboard\n", ""))

    def test_a_required_run_on_the_session_host_is_refused_and_not_run_offscreen(self):
        with edited("keyboard_navigation", 'package = "A1"\nstatus = "required"\nhost = "offscreen"',
                    'package = "A1"\nstatus = "required"\nhost = "session"'):
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
parallel      0
pending       3
  component     1
  harness       1
  scope         1
excluded      1
host      offscreen 3, session 0
view      columns 5, icons 1, list 1
twins     timing 0, a11y 0, rune 0, oracle 0, toolkit 0, deviation 0
package    runs  required  parallel   pending  excluded
A1            1         1         0         0         0
A2            1         0         0         1         0
A6            1         0         0         1         0
A7            1         0         0         1         0
E1            1         0         0         0         1
V7            1         1         0         0         0
V8            1         1         0         0         0
""")
        lines = run_main("inline_renaming", "--counts")[1].splitlines()
        self.assertEqual(lines[0], "== inline_renaming: 2 run(s) in 1 folder(s) ==")
        self.assertEqual(lines[1:4], ["required      0", "parallel      0", "pending       2"])

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


class CheckTests(unittest.TestCase):
    """The reader every selection goes through refuses what the runner's own does."""

    def test_both_readers_accept_and_refuse_what_their_shared_table_says(self):
        table = tomllib.loads((REPOSITORY / "tests/qa/parity_cases.toml").read_text(encoding="utf-8"))
        check = lambda case: qa_parity_check.problems(edit(self, case, table["ledger"]), table["planned"])
        self.assertEqual(qa_parity_check.problems(table["ledger"], table["planned"]), [])
        for case in table["accepted"]:
            with self.subTest(case["name"]):
                self.assertEqual(check(case), [])
        self.assertGreaterEqual(len(table["case"]), 41)
        for case in table["case"]:
            with self.subTest(case["name"]):
                found = check(case)
                self.assertEqual(len(found), 1, found)
                # A problem is the runner's line, word for word; a refusal to read says why.
                self.assertEqual(("problem" in case, "refused" in case).count(True), 1)
                if "problem" in case:
                    self.assertEqual(found[0], case["problem"])
                else:
                    self.assertIn(case["refused"], found[0])
                    self.assertTrue(found[0].startswith(f"{case['file']}.toml: "), found[0])

    def test_the_runners_tests_read_the_same_table_and_spell_the_same_help(self):
        tests = (REPOSITORY / "tests/qa/parity_tests.rs").read_text(encoding="utf-8")
        self.assertIn('const CASES: &str = "tests/qa/parity_cases.toml";', tests)
        reader = (REPOSITORY / "tests/qa/parity.rs").read_text(encoding="utf-8")
        spelt = re.search(r'pub\(crate\) const HELP: &str = "((?:[^"\\]|\\.)*)";', reader, re.S).group(1)
        spelt = re.sub(r"\\\n\s*", "", spelt).replace('\\"', '"')
        self.assertEqual(spelt, qa_parity_check.HELP)

    def test_the_runs_of_a_header_are_the_ones_the_runner_plans(self):
        plain = {"modes": ["columns", "list"]}
        named = {**plain, "variants": [{"id": "us"}, {"id": "ru", "modes": ["icons"]}, {"id": "none", "modes": []}]}
        self.assertEqual(qa_parity_check.runs(plain), [("columns", "columns"), ("list", "list")])
        self.assertEqual(qa_parity_check.runs(named),
                         [("columns", "columns-us"), ("list", "list-us"), ("icons", "icons-ru")])
        with corpus() as (_, scenarios):
            planned = qa_parity_check.planned(scenarios)
        self.assertEqual([(run["folder"], f"{run['scenario']}/{run['run']}", run["gate"]) for run in planned], [
            ("inline_renaming", "rename-with-f2/columns-keyboard", "G3"),
            ("inline_renaming", "rename-with-f2/list-keyboard", None),
            ("keyboard_navigation", "arrow-keys-move-focus/columns-arrows", "G1"),
            ("keyboard_navigation", "context-menu-keeps-selection/columns", "G2"),
            ("keyboard_navigation", "escape-clears-selection/columns", "G2"),
            ("keyboard_navigation", "escape-clears-selection/icons", None),
            ("visual_baselines", "columns-view-baseline/columns", None),
        ])

    def test_a_ledger_with_a_fault_selects_nothing_and_says_which_file_and_row(self):
        required = 'package = "A1"\nstatus = "required"\nhost = "offscreen"\n'
        first = FOLDERS["keyboard_navigation"].split("\n\n")[0]
        swapped = FOLDERS["inline_renaming"].strip().split("\n\n")
        for case, folder, find, put, problem in (
            ("a misspelt status", "keyboard_navigation", required, required.replace("required", "done"),
             "keyboard_navigation.toml: escape-clears-selection/icons has the unknown status 'done'"),
            ("an unknown field", "keyboard_navigation", required, required + 'note = "x"\n',
             "keyboard_navigation.toml: escape-clears-selection/icons has the unknown field note"),
            ("an entry repeated", "keyboard_navigation", first, first + "\n" + first,
             "keyboard_navigation.toml: arrow-keys-move-focus/columns-arrows is listed twice"),
            ("an entry of another folder", "inline_renaming", 'blocker = "A2"\n', 'blocker = "A2"\n' + first + "\n",
             "inline_renaming.toml: arrow-keys-move-focus/columns-arrows is no run of the folder"),
            ("a required run without its host", "keyboard_navigation", required, required.replace('host = "offscreen"\n', ""),
             "keyboard_navigation.toml: escape-clears-selection/icons is required without a host"),
            ("an unknown host", "keyboard_navigation", required, required.replace("offscreen", "window"),
             "keyboard_navigation.toml: escape-clears-selection/icons has the unknown host 'window'"),
            ("no package", "keyboard_navigation", required, required.replace('package = "A1"\n', ""),
             "keyboard_navigation.toml: escape-clears-selection/icons has no package"),
            ("a package the plan does not have", "keyboard_navigation", required, required.replace("A1", "Z9"),
             'keyboard_navigation.toml: escape-clears-selection/icons names "Z9", which is no package of the parity plan'),
            ("a slice package on a pending run", "keyboard_navigation", 'package = "A6"', 'package = "V8"',
             'keyboard_navigation.toml: context-menu-keeps-selection/columns names "V8", which owns only the '
             "required runs of gate G2"),
            ("a required run of the slice demoted", "keyboard_navigation",
             'package = "V8"\ngate = "G2"\nstatus = "required"\nhost = "offscreen"',
             'package = "V8"\ngate = "G2"\nstatus = "pending"\nreason = "scope"\nblocker = "A3"',
             'keyboard_navigation.toml: escape-clears-selection/columns names "V8", which owns only the '
             "required runs of gate G2"),
            ("an entry out of the runner's order", "inline_renaming", FOLDERS["inline_renaming"],
             "\n" + "\n\n".join(reversed(swapped)) + "\n",
             "inline_renaming.toml: rename-with-f2/columns-keyboard is out of the runner's order: it belongs "
             "before rename-with-f2/list-keyboard"),
        ):
            with self.subTest(case), edited(folder, find, put):
                for arguments in ((folder,), (folder, "--counts"), ("all", "--status", "pending"), ("slice",)):
                    status, printed, errors = run_main(*arguments)
                    self.assertEqual((status, printed), (2, ""), arguments)
                    self.assertEqual(errors.splitlines()[1:], [f"  {problem}"], arguments)

    def test_an_unlisted_run_is_refused_with_what_to_do_about_it(self):
        with edited("inline_renaming", FOLDERS["inline_renaming"].strip().split("\n\n")[1], ""):
            status, printed, errors = run_main("all")
        self.assertEqual((status, printed), (2, ""))
        self.assertEqual(errors.splitlines()[1:],
                         ["  inline_renaming.toml: rename-with-f2/list-keyboard is not listed", qa_parity_check.HELP])
        self.assertIn("tests/qa/parity/<its folder>.toml", qa_parity_check.HELP)
        self.assertIn('docs/testing.md, "Who owns a run"', qa_parity_check.HELP)
        self.assertIn("#### Who owns a run", (REPOSITORY / "docs/testing.md").read_text(encoding="utf-8"))


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

    def test_the_committed_ledger_passes_this_reader_as_it_passes_the_runners(self):
        ledger = qa_parity.load()
        planned = qa_parity_check.planned(qa_parity.SCENARIOS)
        self.assertEqual([(run["folder"], run["scenario"], run["run"], run["gate"]) for run in planned],
                         [(run["folder"], run["scenario"], run["run"], run.get("gate")) for run in ledger["run"]])
        slice_owned = [run for run in ledger["run"] if run["package"] in qa_parity_check.SLICE]
        self.assertEqual(len(slice_owned), 36, "the runs the slice took to required stay required")

    def test_the_script_takes_any_selection_and_counts_without_the_image(self):
        script = (REPOSITORY / "scripts/qa-parity.sh").read_text(encoding="utf-8")
        counted = script.index('qa_parity.py" "$selection" "$report"')
        self.assertLess(counted, script.index("qa_container_prepare"))
        self.assertIn('qa_parity.py" "$selection" "$@" "${scenario[@]}"', script)
        for listing in ('list --on "$target"', 'list --status "$status"', "list --status parallel"):
            self.assertIn(listing, script)
        # Both surveys run on Rune alone and say which runs pass there from the shared file.
        self.assertIn("--pending|--parallel)", script)
        for found in ("PASSES ON RUNE, STILL PENDING: ", "PASSES ON RUNE FROM THE SHARED FILE, STILL PARALLEL: "):
            self.assertIn(found, script)
        listed = subprocess.run([str(REPOSITORY / "scripts/qa-parity.sh"), "all", "--twins"],
                                capture_output=True, text=True, check=True)
        self.assertEqual(listed.stdout, qa_parity.twin_listing(qa_parity.load(), "all") + "\n")
        command = [str(REPOSITORY / "scripts/qa-parity.sh")]
        refused = subprocess.run([*command, "no_such_folder"], capture_output=True, text=True, check=False)
        self.assertEqual((refused.returncode, refused.stdout), (2, ""))
        self.assertIn("'no_such_folder' is no selection", refused.stderr)
        counts = subprocess.run([*command, "slice", "--counts"], capture_output=True, text=True, check=True)
        self.assertEqual(counts.stdout, qa_parity.counts(qa_parity.load(), "slice") + "\n")
        # The counts are the whole selection's: an option that narrows a run is refused.
        for narrowed in (["--scenario", "shift-click-range"], ["--pending"], ["--parallel"], ["--target", "rune"]):
            for arguments in (["slice", "--counts", *narrowed], ["slice", *narrowed, "--counts"],
                              ["slice", "--twins", *narrowed]):
                refused = subprocess.run([*command, *arguments], capture_output=True, text=True, check=False)
                self.assertEqual((refused.returncode, refused.stdout), (2, ""), arguments)
                self.assertIn(arguments[1] if arguments[1] in ("--counts", "--twins") else "--counts", refused.stderr)

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
