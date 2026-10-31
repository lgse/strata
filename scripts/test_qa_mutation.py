# SPDX-License-Identifier: MIT

import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

import qa_mutation
from qa_mutation import Mutation, baseline_verdict, chosen_scenarios, mutated_verdict, parse_qa_log

REPOSITORY = Path(__file__).resolve().parents[1]


class QaLogTests(unittest.TestCase):
    def test_result_lines_give_each_run_its_status(self):
        log = "\n".join([
            "RUN copy-leaves-source/columns",
            "RUN copy-leaves-source/icons",
            "RUN copy-leaves-source/list",
            "RUN copy-leaves-source/list-ru",
            "RUN other/columns",
            "PASS copy-leaves-source/columns",
            "thread 'copy_leaves_source' panicked at tests/qa.rs:1:1:",
            "copy-leaves-source/icons failed:",
            "  line 4 {...}: timed out",
            "copy-leaves-source/list could not run:",
            "  launch: no window",
            "KNOWN BUG other/columns still fails as expected (#1)",
            "error: test failed, to rerun pass `--test qa`",
        ])
        self.assertEqual(parse_qa_log(log), {
            "copy-leaves-source/columns": "pass",
            "copy-leaves-source/icons": "fail",
            "copy-leaves-source/list": "error",
            "copy-leaves-source/list-ru": "missing",
            "other/columns": "known bug",
        })
        self.assertEqual(qa_mutation.own_runs("other", parse_qa_log(log)), {"other/columns": "known bug"})


class VerdictTests(unittest.TestCase):
    def test_the_unmodified_runs_must_all_pass(self):
        self.assertTrue(baseline_verdict({"s/columns": "pass", "s/list": "pass"}).caught)
        for status in ("fail", "error", "missing", "known bug"):
            self.assertFalse(baseline_verdict({"s/columns": "pass", "s/list": status}).caught, status)
        self.assertFalse(baseline_verdict({}).caught)

    def test_a_mutation_is_caught_only_by_a_failing_step_and_a_failing_exit(self):
        self.assertTrue(mutated_verdict({"s/columns": "fail", "s/list": "pass"}, 101).caught)
        self.assertFalse(mutated_verdict({"s/columns": "pass", "s/list": "pass"}, 0).caught)
        self.assertFalse(mutated_verdict({"s/columns": "fail"}, 0).caught)
        self.assertFalse(mutated_verdict({"s/columns": "fail", "s/list": "error"}, 101).caught)
        self.assertFalse(mutated_verdict({"s/columns": "fail", "s/list": "missing"}, 101).caught)
        self.assertFalse(mutated_verdict({}, 101).caught, "a build failure runs nothing")


class ScenarioChoiceTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.scenarios = Path(directory.name)
        (self.scenarios / "clipboard").mkdir()
        for name, mutation in (("alpha", "break-alpha"), ("beta", None), ("gamma", "break-gamma")):
            (self.scenarios / "clipboard" / f"{name}.jsonl").write_text(
                json.dumps({"scenario": name, "modes": ["columns"], "mutation": mutation}) + "\n")

    def test_a_name_is_a_mutation_patch_or_else_a_scenario(self):
        chosen = chosen_scenarios(["break-gamma", "alpha"], self.scenarios)
        self.assertEqual([(s.name, s.mutation) for s in chosen],
                         [("gamma", "break-gamma"), ("alpha", "break-alpha")])
        self.assertEqual([s.name for s in chosen_scenarios([], self.scenarios)], ["alpha", "gamma"])
        for name in ("beta", "delta"):
            with self.assertRaises(SystemExit):
                chosen_scenarios([name], self.scenarios)

    def test_every_committed_mutation_belongs_to_exactly_one_scenario(self):
        named = [s.mutation for s in qa_mutation.scenarios() if s.mutation]
        self.assertEqual(sorted(named), sorted(set(named)))
        self.assertEqual(sorted(named), sorted(path.stem for path in qa_mutation.MUTATIONS.glob("*.patch")))


class MutationTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.repository = self.root / "repository"
        (self.repository / "src").mkdir(parents=True)
        (self.repository / "src/main.rs").write_text("fn main() {}\n")
        for command in (["init", "-q"], ["add", "."],
                        ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "base"]):
            subprocess.run(["git", "-C", str(self.repository), *command], check=True)
        self.mutations = self.root / "mutations"
        self.mutations.mkdir()
        (self.mutations / "panic.patch").write_text(
            "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-fn main() {}\n+fn main() { panic!() }\n")

    def test_a_patch_applies_for_the_block_and_is_restored_after(self):
        source = self.repository / "src/main.rs"
        with Mutation("panic", self.repository, self.mutations):
            self.assertEqual(source.read_text(), "fn main() { panic!() }\n")
        self.assertEqual(source.read_text(), "fn main() {}\n")

    def test_a_patch_never_overwrites_uncommitted_changes(self):
        source = self.repository / "src/main.rs"
        source.write_text("fn main() { work() }\n")
        with self.assertRaises(SystemExit):
            with Mutation("panic", self.repository, self.mutations):
                pass
        self.assertEqual(source.read_text(), "fn main() { work() }\n")


class CiWorkflowTests(unittest.TestCase):
    def jobs(self):
        workflow = (REPOSITORY / ".github/workflows/ci.yml").read_text()
        return dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)",
                               workflow.split("\njobs:\n", 1)[1], re.M | re.S))

    def test_both_release_marker_checks_grep_a_prefix_of_the_qa_usage_constant(self):
        source = (REPOSITORY / "src/qa.rs").read_text()
        usage = re.search(r'^const USAGE: &str = "([^"]+)";$', source, re.M).group(1)
        self.assertIn("return Err(USAGE.to_owned());", source)
        release = (REPOSITORY / ".github/workflows/release.yml").read_text()
        check = release.split("- name: Check the release binary has no QA mode", 1)[1].split("\n      - name:", 1)[0]
        pattern = re.search(r'if strings "target/\$TARGET/release/strata" \| grep -F "([^"]+)"; then', check).group(1)
        self.assertTrue(usage.startswith(pattern), (usage, pattern))
        self.assertIn(f'| grep -F "{pattern}"', self.jobs()["qa"])


if __name__ == "__main__":
    unittest.main()
