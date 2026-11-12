# SPDX-License-Identifier: MIT
"""The labelled twins of the parity ledger, through the scripts' reader: the pair table both
readers are held to, and how a selection, the counts and the listing treat a parallel run."""

import contextlib
import re
import tomllib
import unittest
import unittest.mock
from pathlib import Path

import qa_parity
import qa_parity_mutation
import qa_parity_twins
import test_qa_parity
from qa_parity import selected
from test_qa_parity import SCENARIOS, edit, edited, load, run_main

REPOSITORY = Path(__file__).resolve().parents[1]

# A labelled pair for the corpus: escape-clears-selection cannot pass in Icons on Rune under
# the `a11y` exception, so its twin stands in for that run there.
TWIN = "escape-clears-selection-rune"
LABEL = {"exception": "a11y", "ref": "lgse/strata#2201"}
PAIR = {
    "keyboard_navigation/escape-clears-selection": {
        "modes": ["columns", "icons"], "parity": {"twin": TWIN, **LABEL}},
    f"keyboard_navigation/{TWIN}": {
        "modes": ["icons"], "targets": ["rune"],
        "parity": {"twin": "escape-clears-selection", **LABEL, "differs": "line 3 waits for the tile"},
        "steps": ['{"key":"Escape"}', '{"wait":{"focused":"a"}}']},
}
REQUIRED = 'package = "A1"\nstatus = "required"\nhost = "offscreen"\n'
PARALLEL = ('package = "A1"\nstatus = "parallel"\nexception = "a11y"\n'
            f'twin = "{TWIN}/icons"\nref = "lgse/strata#2201"\n')


@contextlib.contextmanager
def paired(row=PARALLEL, patches=(TWIN,)):
    """The corpus with the pair: its two headers, the entry `row` and the twin's patch."""

    with unittest.mock.patch.dict(SCENARIOS, PAIR), unittest.mock.patch.object(
            test_qa_parity, "PATCHES", list(patches)), edited("keyboard_navigation", REQUIRED, row):
        yield


class TwinTests(unittest.TestCase):
    """A parallel run is run as a required one is, by another route on Rune."""

    def test_a_parallel_run_is_its_scenario_on_gtk_and_its_twin_on_rune(self):
        with paired():
            ledger = load()
            self.assertEqual(run_main("keyboard_navigation", "--on", "gtk"), (0, (
                "arrow-keys-move-focus/columns-arrows\nescape-clears-selection/columns\n"
                "escape-clears-selection/icons\n"), ""))
            self.assertEqual(run_main("keyboard_navigation", "--on", "rune"), (0, (
                "arrow-keys-move-focus/columns-arrows\nescape-clears-selection/columns\n"
                f"{TWIN}/icons\n"), ""))
        self.assertEqual(len(ledger["run"]), 7, "a twin's run has no entry of its own")
        self.assertEqual(selected(ledger, "A1", "parallel"), ["escape-clears-selection/icons"])
        self.assertEqual(selected(ledger, "A1", "required"), [])
        # A gate has only Columns runs, and a survey only pending ones.
        self.assertEqual(qa_parity.deciding(ledger, "g2", "rune"), ["escape-clears-selection/columns"])
        self.assertNotIn(f"{TWIN}/icons", selected(ledger, "all", "pending"))

    def test_either_name_of_a_pair_narrows_a_selection_to_it(self):
        with paired():
            ledger = load()
        for scenario, gtk, rune in (
            (TWIN, ["escape-clears-selection/icons"], [f"{TWIN}/icons"]),
            ("escape-clears-selection", ["escape-clears-selection/columns", "escape-clears-selection/icons"],
             ["escape-clears-selection/columns", f"{TWIN}/icons"]),
        ):
            with self.subTest(scenario):
                self.assertEqual(qa_parity.deciding(ledger, "all", "gtk", scenario), gtk)
                self.assertEqual(qa_parity.deciding(ledger, "all", "rune", scenario), rune)

    def test_a_parallel_run_whose_twin_needs_the_session_host_is_refused(self):
        with paired(row=PARALLEL.replace('status = "parallel"\n', 'status = "parallel"\nhost = "session"\n')):
            self.assertEqual(qa_parity.needs_session(load(), "A1"), ["escape-clears-selection/icons"])
            for target in ("gtk", "rune"):
                status, printed, errors = run_main("A1", "--on", target)
                self.assertEqual((status, printed), (3, ""))
                self.assertIn("escape-clears-selection/icons", errors)
        with paired():
            self.assertEqual(qa_parity.needs_session(load(), "all"), [])

    def test_the_twins_are_listed_with_what_a_review_or_a_checkpoint_audits(self):
        with paired():
            self.assertEqual(run_main("A1", "--twins"), (0, f"""\
== A1: 1 twin(s) for 1 parallel run(s) ==
{TWIN}  a11y  lgse/strata#2201  A1
  stands in for  escape-clears-selection/icons
  differs        line 3 waits for the tile
  steps          1 the original's, 1 its own, 0 of the original's dropped
""", ""))
            self.assertEqual(run_main("g2", "--twins"), (0, "== g2: 0 twin(s) for 0 parallel run(s) ==\n", ""))
        # Steps are counted over the runs a twin holds, each as often as it occurs, and a
        # step that differs only in its message or its timeout is the original's.
        shared = [(2, {"key": "Down"}), (3, {"key": "Down"}), (4, {"fs": {"path": "a"}})]
        own = qa_parity_twins.run_steps(
            [(2, {"key": "Down", "message": "x", "timeout_ms": 5}), (3, {"click": "a", "modes": ["icons"]}),
             (4, {"type": "$name"}), (5, {"key": "Up", "variants": ["us"]})], "icons", {"id": "ru", "vars": {"name": "b"}})
        self.assertEqual(own, [(2, {"key": "Down"}), (3, {"click": "a"}), (4, {"type": "b"})])
        self.assertEqual(qa_parity_twins.tally(shared, own), (1, 2, 2))

    def test_a_twins_patch_is_named_for_the_twin_and_its_runs_guard_it(self):
        with paired():
            ledger = load()
        self.assertEqual(qa_parity_mutation.guards(TWIN, ledger), [f"{TWIN}/icons"])
        self.assertEqual(qa_parity_mutation.guards("escape-clears-selection", ledger),
                         ["escape-clears-selection/columns"])
        # A scenario whose every run is parallel runs nothing of its own on Rune: the patch
        # that proves the pair is the twin's.
        whole = {"run": [{"folder": "keys", "scenario": "moves", "run": "columns", "package": "A7",
                          "status": "parallel", "twin": "moves-rune/columns"}]}
        self.assertEqual(qa_parity_mutation.unguarded(["moves", "moves-rune"], whole), ["moves"])


class TwinCountTests(unittest.TestCase):
    def test_a_twin_moves_a_run_to_parallel_and_is_counted_by_its_exception_not_as_a_run(self):
        before = run_main("all", "--counts")[1].splitlines()
        with paired():
            after = run_main("all", "--counts")[1].splitlines()
        self.assertEqual(after[0], before[0], "the twin's own run is no run of the ledger")
        self.assertEqual(after[1:4], ["required      2", "parallel      1", "  a11y          1"])
        self.assertIn("twins     timing 0, a11y 1, rune 0, oracle 0, toolkit 0, deviation 0", after)
        self.assertIn("A1            1         0         1         0         0", after)
        # A parallel run that names no host is counted on the offscreen one, where its twin runs.
        self.assertIn("host      offscreen 3, session 0", after)
        # A twin that holds two runs of its scenario is one twin.
        runs = [{"status": "parallel", "exception": "timing", "twin": f"moves-rune/{mode}"}
                for mode in ("columns", "list")]
        self.assertEqual(qa_parity._twins(runs), {"timing": 1})


class PairCheckTests(unittest.TestCase):
    """The reader every selection goes through refuses the pairs the runner's own does."""

    def pair_outcome(self, pairs, case=None):
        """What this reader makes of the pair table's corpus after a case's edits."""

        case = case or {}
        texts = {"header": dict(pairs["scenarios"]), "file": dict(pairs["ledger"])}
        for change in case.get("edit", []):
            which = "header" if "header" in change else "file"
            texts[which] = edit(self, {**change, "file": change[which], "name": case["name"]}, texts[which])
        facts = [qa_parity_twins.facts(*path.split("/"), text) for path, text in texts["header"].items()]
        corpus = qa_parity_twins.assemble(facts, set(case.get("patches", pairs["patches"])), set(pairs["packet"]))
        return qa_parity_twins.faults(texts["file"], corpus)

    def test_both_readers_accept_and_refuse_the_pairs_their_shared_table_says(self):
        pairs = tomllib.loads((REPOSITORY / "tests/qa/parity_pair_cases.toml").read_text(encoding="utf-8"))
        self.assertEqual(self.pair_outcome(pairs), [])
        for case in pairs["accepted"]:
            with self.subTest(case["name"]):
                self.assertEqual(self.pair_outcome(pairs, case), [])
        self.assertGreaterEqual(len(pairs["case"]), 60)
        for case in pairs["case"]:
            with self.subTest(case["name"]):
                found = self.pair_outcome(pairs, case)
                self.assertEqual(len(found), 1, found)
                self.assertEqual(("problem" in case, "refused" in case).count(True), 1)
                if "problem" in case:
                    self.assertEqual(found[0], case["problem"])
                else:
                    self.assertIn(case["refused"], found[0])

    def test_this_reader_knows_the_header_keys_the_runner_has_and_no_other(self):
        source = (REPOSITORY / "tests/qa/scenario.rs").read_text(encoding="utf-8")
        for struct, shapes in (("Header", qa_parity_twins.SHAPES), ("Variant", qa_parity_twins.VARIANT_SHAPES)):
            body = re.search(rf"pub\(crate\) struct {struct} \{{\n(.*?)\n\}}", source, re.S).group(1)
            self.assertEqual(re.findall(r"^    pub\(crate\) (\w+):", body, re.M), list(shapes), struct)
        tests = (REPOSITORY / "tests/qa/twin_tests.rs").read_text(encoding="utf-8")
        self.assertIn('const CASES: &str = "tests/qa/parity_pair_cases.toml";', tests)
        label = (REPOSITORY / "tests/qa/twins.rs").read_text(encoding="utf-8")
        body = re.search(r"pub\(crate\) struct Label \{\n(.*?)\n\}", label, re.S).group(1)
        fields = re.findall(r"^    pub\(crate\) (\w+):", body, re.M)
        self.assertEqual([{"reference": "ref"}.get(field, field) for field in fields], list(qa_parity_twins.LABEL))
        for constant, mine in (("UNCOMPARED", qa_parity_twins.UNCOMPARED),):
            spelt = re.search(rf"const {constant}: \[&str; \d+\] = \[(.*?)\];", label, re.S).group(1)
            self.assertEqual(tuple(re.findall(r'"(\w+)"', spelt)), mine)

    def test_a_pair_with_a_fault_selects_nothing_and_says_what_to_do(self):
        twin = "scenarios/keyboard_navigation/escape-clears-selection-rune.jsonl"
        for case, pair, problem in (
            ("a twin with no patch", paired(patches=()),
             f"{twin} has no proof that it can fail: add tests/qa/parity/mutations/{TWIN}.patch, which "
             "breaks the behaviour in crates/strata-app, and check it with ./scripts/qa-parity.sh mutations"),
            ("a twin whose run the ledger still calls required", paired(row=REQUIRED),
             f"keyboard_navigation.toml: escape-clears-selection/icons is required, and {TWIN}/icons stands "
             "in for it: a run with a twin is parallel, with the exception, the twin and the reference"),
        ):
            with self.subTest(case), pair:
                for arguments in (("A1",), ("all", "--counts"), ("slice", "--on", "rune")):
                    status, printed, errors = run_main(*arguments)
                    self.assertEqual((status, printed), (2, ""), arguments)
                    self.assertEqual(errors.splitlines()[1:], [f"  {problem}"], arguments)


if __name__ == "__main__":
    unittest.main()
