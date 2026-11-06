# SPDX-License-Identifier: MIT

import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import quality_ci as quality


class QualityCiTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bundle = self.root / "bundle"
        self.bundle.mkdir()
        self.reports = self.root / "reports"
        self.record = self.root / "record.jsonl"
        self.executable = self.bundle / "test-0"
        self.executable.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "names = ['a', 'b', 'c', 'd', 'ignored']\n"
            "args = sys.argv[1:]\n"
            "filters = [arg for arg in args if not arg.startswith('-') and arg != 'unstable-options']\n"
            "selected = [name for name in names if not filters or name in filters]\n"
            "if '--ignored' in args: selected = [name for name in selected if name == 'ignored']\n"
            "if '--list' in args:\n"
            " for name in selected: print(name + ': test')\n"
            "else:\n"
            " if '-Z' in args and 'RUSTC_BOOTSTRAP' not in os.environ:\n"
            "  sys.exit('the option `Z` is only accepted on the nightly compiler')\n"
            " with open(os.environ['RECORD'], 'a') as record:\n"
            "  record.write(json.dumps(dict(display=os.environ.get('DISPLAY'), selected=selected,\n"
            "                               gtk=os.environ.get('STRATA_REQUIRE_GTK_TESTS'))) + '\\n')\n"
            " if os.environ.get('FAIL_TEST') in selected:\n"
            "  print(\"thread 'x' panicked at src/fixture.rs:1:1:\\nboom\")\n"
            "  sys.exit(101)\n"
            " if '--report-time' in args:\n"
            "  for name in selected:\n"
            "   if name == 'b':\n"
            "    print('test b ... \\nrunning 1 test\\ntest b ... ok\\n\\nok <2.00s>')\n"
            "   elif name != 'ignored':\n"
            "    print(f'test {name} ... ok <0.20s>')\n"
            " if os.environ.get('SUMMARY') != 'none':\n"
            "  ran = 0 if os.environ.get('SUMMARY') == 'empty' else len(selected)\n"
            "  ignored = int('ignored' in selected and ran > 0)\n"
            "  print(f'test result: ok. {ran-ignored} passed; 0 failed; {ignored} ignored; 0 measured; 0 filtered out; finished in 0.01s')\n"
        )
        self.executable.chmod(0o755)
        self.plan = dict(version=1, commit="fixture", source_key="source", image_key="image", binaries=[dict(
            file="test-0", target="fixture", sha256=quality.digest(self.executable),
            tests=["a", "b", "c", "d", "ignored"], ignored=["ignored"],
            shards=[["a"], ["b", "c", "d", "ignored"]],
        )])
        self.plan_path = self.bundle / "plan.json"
        self.save_plan()
        scripts = self.root / "scripts"
        scripts.mkdir()
        (scripts / "quality-durations.json").write_text("{}")
        tools = self.root / "tools"
        tools.mkdir()
        # The fakes keep only what isolation provides: a display per lane and a wrapped bus.
        (tools / "xvfb-run").write_text(
            f"#!{sys.executable}\n"
            "import os, sys\n"
            "args = sys.argv[1:]\n"
            "assert args[0] == '-n' and args[2] == '-s' and args[4] == 'dbus-run-session', args\n"
            "os.environ['DISPLAY'] = ':' + args[1]\n"
            "os.execvp(args[4], args[4:])\n")
        (tools / "dbus-run-session").write_text(
            f"#!{sys.executable}\n"
            "import os, sys\n"
            "args = sys.argv[sys.argv.index('--') + 1:]\n"
            "os.execvp(args[0], args)\n")
        for tool in tools.iterdir():
            tool.chmod(0o755)
        for name, value in (("ROOT", self.root), ("BUNDLE", self.bundle), ("REPORTS", self.reports),
                            ("LOCAL", self.root / "local")):
            patcher = patch.object(quality, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        for patcher in (patch.object(quality, "ISOLATED_TEST", "a"),
                        patch.object(quality, "SHARDS", 2),
                        patch.dict(quality.os.environ, STRATA_QUALITY_COMMIT="fixture", RECORD=str(self.record),
                                   PATH=f"{tools}:{quality.os.environ['PATH']}"),
                        patch.object(quality, "test_source_key", return_value="source"),
                        patch.object(quality, "image_key", return_value="image")):
            patcher.start()
            self.addCleanup(patcher.stop)

    def save_plan(self):
        self.plan_path.write_text(json.dumps(self.plan))

    def runs(self):
        return [json.loads(line) for line in self.record.read_text().splitlines()]

    def run_all(self, lanes=1):
        with contextlib.redirect_stdout(io.StringIO()):
            for shard in range(quality.SHARDS):
                quality.run(shard, lanes)

    def verify(self):
        with contextlib.redirect_stdout(io.StringIO()):
            quality.verify(self.plan_path, self.reports)

    def test_real_inventory_selection_execution_and_aggregation(self):
        self.run_all()
        self.verify()
        report = json.loads((self.reports / "shard-0.json").read_text())
        self.assertEqual(report["binaries"][0]["passed"], ["a"])
        self.assertEqual(report["binaries"][0]["ignored"], [])
        report = json.loads((self.reports / "shard-1.json").read_text())
        self.assertEqual(report["binaries"][0]["ignored"], ["ignored"])

    def test_mixed_shard_runs_isolated_test_in_a_separate_process(self):
        self.plan["binaries"][0]["shards"] = [["a", "b", "ignored"], ["c", "d"]]
        self.save_plan()
        self.run_all()
        self.verify()
        self.assertEqual([run["selected"] for run in self.runs()], [["a"], ["b", "ignored"], ["c", "d"]])
        report = json.loads((self.reports / "shard-0.json").read_text())
        self.assertEqual(report["binaries"][0]["passed"], ["a", "b"])
        self.assertEqual(report["binaries"][0]["ignored"], ["ignored"])

    def test_lanes_split_a_shard_across_private_displays_under_one_receipt(self):
        self.run_all(lanes=2)
        self.verify()
        runs = self.runs()
        self.assertEqual(sorted(name for run in runs for name in run["selected"]), self.plan["binaries"][0]["tests"])
        self.assertTrue(all(run["gtk"] == "1" for run in runs))
        self.assertNotIn(None, [run["display"] for run in runs])
        shard_1 = [run for run in runs if "a" not in run["selected"]]
        self.assertEqual(len(shard_1), 2)
        self.assertNotEqual(shard_1[0]["display"], shard_1[1]["display"])

    def test_partition_balances_both_shards_using_timing_hints(self):
        names = ["a", *[f"test_{index}" for index in range(20)]]
        for isolated_duration in (0, 154, 1000):
            with self.subTest(isolated_duration=isolated_duration):
                durations = {"a": isolated_duration, "test_0": 25}
                shards = quality.partition(names, durations)
                self.assertEqual(shards, quality.partition(list(reversed(names)), durations))
                self.assertEqual(sorted(test for shard in shards for test in shard), sorted(names))
                loads = [sum(durations.get(name, 1) for name in shard) for shard in shards]
                self.assertLessEqual(abs(loads[0] - loads[1]), max(durations.values()))
                self.assertTrue(all(shards))
        shards = quality.partition(names, {})
        self.assertLessEqual(abs(len(shards[0]) - len(shards[1])), 1)

    def test_other_targets_use_both_shards(self):
        binary = copy.deepcopy(self.plan["binaries"][0])
        binary.update(file="test-1", tests=["e", "f", "g"], ignored=[],
                      shards=quality.partition(["e", "f", "g"], {}))
        self.assertEqual(binary["shards"], [["e", "g"], ["f"]])
        self.plan["binaries"].append(binary)
        quality.validate_plan(self.plan)

    def test_plan_rejects_missing_ignored_or_duplicate_isolated_test(self):
        def remove(plan):
            plan["binaries"][0]["tests"].remove("a")
            plan["binaries"][0]["shards"][0].remove("a")

        def ignore(plan):
            plan["binaries"][0]["ignored"].append("a")

        def duplicate(plan):
            binary = copy.deepcopy(plan["binaries"][0])
            binary["file"] = "test-1"
            plan["binaries"].append(binary)

        for mutate in (remove, ignore, duplicate):
            with self.subTest(mutate=mutate.__name__):
                plan = copy.deepcopy(self.plan)
                mutate(plan)
                with self.assertRaises(ValueError):
                    quality.validate_plan(plan)

    def test_plan_rejects_missing_duplicate_extra_and_empty_assignments(self):
        mutations = [
            lambda plan: plan["binaries"][0]["shards"][0].append("a"),
            lambda plan: plan["binaries"][0]["shards"][0].append("extra"),
            lambda plan: plan["binaries"][0]["shards"][0].remove("a"),
            lambda plan: plan["binaries"][0].update(file="../test-0"),
            lambda plan: plan["binaries"].append(copy.deepcopy(plan["binaries"][0])),
            lambda plan: plan["binaries"][0]["ignored"].extend(["b", "c", "d"]),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                plan = copy.deepcopy(self.plan)
                mutate(plan)
                with self.assertRaises(ValueError):
                    quality.validate_plan(plan)

    def test_reports_reject_missing_duplicate_stale_and_bad_coverage(self):
        self.run_all()
        path = self.reports / "shard-0.json"
        original = path.read_text()
        mutations = [
            lambda report: report.update(shard=1),
            lambda report: report.update(plan_sha256="stale"),
            lambda report: report["binaries"][0]["passed"].append("a"),
            lambda report: report["binaries"][0]["passed"].clear(),
            lambda report: report["binaries"][0]["ignored"].append("a"),
            lambda report: report["binaries"][0]["passed"].append("extra"),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                report = json.loads(original)
                mutate(report)
                path.write_text(json.dumps(report))
                with self.assertRaises(ValueError):
                    self.verify()
        path.unlink()
        with self.assertRaises(ValueError):
            self.verify()
        path.write_text(original)
        (self.reports / "shard-extra.json").write_text(original)
        with self.assertRaises(ValueError):
            self.verify()

    def test_wrong_binary_checkout_inventory_and_shard_fail(self):
        for field, value in (("sha256", "bad"), ("tests", ["wrong"]), ("ignored", [])):
            with self.subTest(field=field):
                original = copy.deepcopy(self.plan)
                self.plan["binaries"][0][field] = value
                self.save_plan()
                with self.assertRaises(ValueError):
                    quality.run(0)
                self.plan = original
        self.plan["commit"] = "other"
        self.save_plan()
        with self.assertRaises(ValueError):
            quality.run(0)
        for shard in (-1, quality.SHARDS, None):
            with self.assertRaises(ValueError):
                quality.run(shard)

    def test_failed_or_zero_test_process_cannot_publish_success_receipt(self):
        self.run_all()
        for env in (dict(FAIL_TEST="a"), dict(SUMMARY="none"), dict(SUMMARY="empty")):
            with (self.subTest(env=env), patch.dict(quality.os.environ, env),
                  contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError)):
                quality.run(0)
            self.assertFalse((self.reports / "shard-0.json").exists())

    def test_one_failed_lane_fails_the_shard_after_the_others_finish(self):
        output = io.StringIO()
        with (patch.dict(quality.os.environ, FAIL_TEST="b"), contextlib.redirect_stdout(output),
              self.assertRaises(ValueError)):
            quality.run(1, lanes=2)
        self.assertFalse((self.reports / "shard-1.json").exists())
        self.assertIn("lane 0 FAILED", output.getvalue())
        self.assertIn("lane 1 passed", output.getvalue())
        self.assertIn("panicked at src/fixture.rs:1:1:\nboom", output.getvalue())

    def test_build_collects_all_cargo_test_artifacts_once(self):
        artifact = self.root / "compiled-test"
        artifact.write_bytes(self.executable.read_bytes())
        artifact.chmod(0o755)
        message = dict(reason="compiler-artifact", profile=dict(test=True),
                       executable=str(artifact), target=dict(name="unit"))
        real_run = subprocess.run
        commands = []

        def run(command, **kwargs):
            if command[0] == "cargo":
                commands.append(command)
                return subprocess.CompletedProcess(command, 0, json.dumps(message) + "\n")
            return real_run(command, **kwargs)

        with patch.object(quality.subprocess, "run", side_effect=run):
            quality.build()
        self.assertEqual(len(commands), 1)
        self.assertTrue({"--locked", "--all-targets", "--all-features", "--no-run"} <= set(commands[0]))
        plan = json.loads(self.plan_path.read_text())
        self.assertEqual(plan["binaries"][0]["tests"], self.plan["binaries"][0]["tests"])
        self.assertEqual(plan["binaries"][0]["ignored"], ["ignored"])
        self.assertEqual(plan["binaries"][0]["shards"],
                         quality.partition(plan["binaries"][0]["tests"], {}))

    def run_local(self, lanes=2, timings=False):
        artifact = self.root / "compiled-test"
        artifact.write_bytes(self.executable.read_bytes())
        artifact.chmod(0o755)
        message = dict(reason="compiler-artifact", profile=dict(test=True),
                       executable=str(artifact), target=dict(name="unit"))
        real_run = subprocess.run

        def run(command, **kwargs):
            if command[0] == "cargo":
                return subprocess.CompletedProcess(command, 0, json.dumps(message) + "\n")
            return real_run(command, **kwargs)

        output = io.StringIO()
        with patch.object(quality.subprocess, "run", side_effect=run), contextlib.redirect_stdout(output):
            quality.local(lanes, timings)
        return output.getvalue()

    def test_local_runs_the_whole_suite_as_one_shard_of_concurrent_lanes(self):
        output = self.run_local()
        self.assertIn("Verified 4 passed tests exactly once; 1 explicitly ignored by libtest", output)
        runs = self.runs()
        self.assertEqual(sorted(name for run in runs for name in run["selected"]), self.plan["binaries"][0]["tests"])
        self.assertEqual(len({run["display"] for run in runs}), len(runs))
        self.assertEqual(len(json.loads((self.root / "local/bundle/plan.json").read_text())["binaries"][0]["shards"]), 1)

    def test_refreshed_timings_weigh_gtk_children_fully_and_the_rest_by_parallelism(self):
        self.run_local(timings=True)
        weights = json.loads((self.root / "scripts/quality-durations.json").read_text())
        self.assertEqual(weights, {"a": 0.05, "b": 2.0, "c": 0.05, "d": 0.05})

    def test_failure_excerpt_keeps_failed_test_panics_and_drops_expected_ones(self):
        log = "\n".join([
            "thread 'helper' (1) panicked at src/writer.rs:1:1:", "an injected failure", "",
            "thread 'module::fails' (2) panicked at src/module.rs:2:2:", "real cause", "",
            *[f"test module::passes_{index} ... ok" for index in range(40)],
            "test module::fails ... FAILED",
        ])
        excerpt = quality.failure_excerpt(log)
        self.assertIn("real cause", excerpt)
        self.assertNotIn("an injected failure", excerpt)
        self.assertTrue(excerpt.endswith("test module::fails ... FAILED"))

    def test_non_positive_lane_counts_are_rejected_before_any_work(self):
        for lanes in (0, -1):
            with self.subTest(lanes=lanes), patch.object(quality, "build") as build:
                with self.assertRaises(ValueError):
                    quality.local(lanes)
                with self.assertRaises(ValueError):
                    quality.run(0, lanes)
                build.assert_not_called()
            self.assertFalse(self.record.exists())


if __name__ == "__main__":
    unittest.main()
