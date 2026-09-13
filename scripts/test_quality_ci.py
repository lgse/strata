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
        self.executable = self.bundle / "test-0"
        self.executable.write_text(
            f"#!{sys.executable}\n"
            "import sys\n"
            "names = ['a', 'b', 'c', 'd', 'ignored']\n"
            "args = sys.argv[1:]\n"
            "filters = [arg for arg in args if not arg.startswith('--')]\n"
            "selected = [name for name in names if not filters or name in filters]\n"
            "if '--ignored' in args: selected = [name for name in selected if name == 'ignored']\n"
            "if '--list' in args:\n"
            " for name in selected: print(name + ': test')\n"
            "else:\n"
            " ignored = int('ignored' in selected)\n"
            " print(f'test result: ok. {len(selected)-ignored} passed; 0 failed; {ignored} ignored; 0 measured; 0 filtered out; finished in 0.01s')\n"
        )
        self.executable.chmod(0o755)
        self.plan = dict(version=1, commit="fixture", source_key="source", image_key="image", binaries=[dict(
            file="test-0", target="fixture", sha256=quality.digest(self.executable),
            tests=["a", "b", "c", "d", "ignored"], ignored=["ignored"],
            shards=[["a"], ["b", "ignored"], ["c"], ["d"]],
        )])
        (self.bundle / "strata-media-helper").write_bytes(b"test helper")
        self.plan["helper_sha256"] = quality.digest(self.bundle / "strata-media-helper")
        self.plan_path = self.bundle / "plan.json"
        self.save_plan()
        for name, value in (("ROOT", self.root), ("BUNDLE", self.bundle), ("REPORTS", self.reports)):
            patcher = patch.object(quality, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        for patcher in (patch.object(quality, "ISOLATED_TEST", "a"),
                        patch.dict(quality.os.environ, STRATA_QUALITY_COMMIT="fixture"),
                        patch.object(quality, "test_source_key", return_value="source"),
                        patch.object(quality, "image_key", return_value="image")):
            patcher.start()
            self.addCleanup(patcher.stop)

    def save_plan(self):
        self.plan_path.write_text(json.dumps(self.plan))

    def run_all(self):
        with contextlib.redirect_stdout(io.StringIO()):
            for shard in range(quality.SHARDS):
                quality.run(shard)

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

    def test_partition_reserves_shard_zero_independently_of_timing_hints(self):
        names = ["a", *[f"test_{index}" for index in range(20)]]
        for isolated_duration in (0, 154, 1000):
            with self.subTest(isolated_duration=isolated_duration):
                durations = {"a": isolated_duration, "test_0": 25}
                shards = quality.partition(names, durations)
                self.assertEqual(shards, quality.partition(list(reversed(names)), durations))
                self.assertEqual(sorted(test for shard in shards for test in shard), sorted(names))
                self.assertEqual(shards[0], ["a"])
                self.assertEqual(shards[1], ["test_0"])
                self.assertTrue(all(shards))
        self.assertEqual(quality.partition(names, {})[0], ["a"])

    def test_other_targets_never_fill_the_reserved_shard(self):
        binary = copy.deepcopy(self.plan["binaries"][0])
        binary.update(file="test-1", tests=["e", "f", "g"], ignored=[],
                      shards=quality.partition(["e", "f", "g"], {}))
        self.assertEqual(binary["shards"], [[], ["e"], ["f"], ["g"]])
        self.plan["binaries"].append(binary)
        quality.validate_plan(self.plan)

    def test_plan_rejects_mixing_moving_missing_ignored_or_duplicate_isolated_test(self):
        def mix(plan):
            plan["binaries"][0]["shards"][0].append("b")
            plan["binaries"][0]["shards"][1].remove("b")

        def move(plan):
            plan["binaries"][0]["shards"][0].remove("a")
            plan["binaries"][0]["shards"][1].append("a")

        def remove(plan):
            plan["binaries"][0]["tests"].remove("a")
            plan["binaries"][0]["shards"][0].remove("a")

        def ignore(plan):
            plan["binaries"][0]["ignored"].append("a")

        def duplicate(plan):
            binary = copy.deepcopy(plan["binaries"][0])
            binary["file"] = "test-1"
            plan["binaries"].append(binary)

        for mutate in (mix, move, remove, ignore, duplicate):
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
            lambda plan: plan["binaries"][0]["ignored"].append("d"),
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
        for shard in (-1, 4, None):
            with self.assertRaises(ValueError):
                quality.run(shard)

    def test_failed_or_zero_test_process_cannot_publish_success_receipt(self):
        self.run_all()
        real_run = subprocess.run
        for code, output in ((42, ""), (0, ""), (0, "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n")):
            def run(command, **kwargs):
                if "--nocapture" in command:
                    return subprocess.CompletedProcess(command, code, output)
                return real_run(command, **kwargs)
            with self.subTest(code=code, output=output), patch.object(quality.subprocess, "run", side_effect=run):
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                    quality.run(0)
                self.assertFalse((self.reports / "shard-0.json").exists())

    def test_build_collects_all_cargo_test_artifacts_once(self):
        scripts = self.root / "scripts"
        scripts.mkdir()
        (scripts / "quality-durations.json").write_text("{}")
        artifact = self.root / "compiled-test"
        artifact.write_bytes(self.executable.read_bytes())
        artifact.chmod(0o755)
        message = dict(reason="compiler-artifact", profile=dict(test=True),
                       executable=str(artifact), target=dict(name="unit"))
        real_run = subprocess.run
        commands = []
        helper = self.root / "target/debug/strata-media-helper"
        helper.parent.mkdir(parents=True)
        helper.write_bytes(b"compiled media helper")

        def run(command, **kwargs):
            if command[0] == "cargo":
                commands.append(command)
                return subprocess.CompletedProcess(command, 0, json.dumps(message) + "\n")
            return real_run(command, **kwargs)

        with patch.object(quality.subprocess, "run", side_effect=run):
            quality.build()
        self.assertEqual(len(commands), 2)
        self.assertEqual(commands[0], ["cargo", "build", "--locked", "-p", "strata-media-helper"])
        self.assertTrue({"--locked", "--all-targets", "--all-features", "--no-run"} <= set(commands[1]))
        plan = json.loads(self.plan_path.read_text())
        self.assertEqual(plan["binaries"][0]["tests"], self.plan["binaries"][0]["tests"])
        self.assertEqual(plan["binaries"][0]["ignored"], ["ignored"])
        self.assertEqual(plan["binaries"][0]["shards"][0], ["a"])


if __name__ == "__main__":
    unittest.main()
