# SPDX-License-Identifier: GPL-3.0-or-later

from copy import deepcopy
from datetime import datetime, timezone
import io
import json
from pathlib import Path
import random
import sys
import tempfile
import unittest
from unittest.mock import patch

from e2e_bundle import create, image_key, verify
from e2e_ci import check_budget, critical_path, main as ci_main, workflow_jobs

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests/e2e"))
from harness.sharding import make_plan, validate_plan, verify_reports


def inventory(count):
    return [{"nodeid": f"tests/test_sample.py::test_case[{index}]", "group": None}
            for index in range(count)]


def passing_reports(plan):
    return [{"shard": shard["index"], "exitstatus": 0, "inventory": plan["inventory"],
             "tests": {nodeid: {"seconds": 2.0, "outcomes": dict.fromkeys(
                 ("setup", "call", "teardown"), "passed")} for nodeid in shard["nodeids"]}}
            for shard in plan["shards"]]


class ShardingTests(unittest.TestCase):
    def test_plan_covers_every_parameter_exactly_once_and_keeps_groups_together(self):
        tests = inventory(150)
        for test in tests[:6]:
            test["group"] = "visual-baselines"
        plan = make_plan(tests, {})
        validate_plan(plan, tests)
        members = {test["nodeid"] for test in tests[:6]}
        self.assertEqual(sum(members <= set(shard["nodeids"]) for shard in plan["shards"]), 1)
        self.assertTrue(all(shard["estimated_seconds"] <= 40 for shard in plan["shards"]))

    def test_input_order_and_duration_key_order_do_not_change_assignments(self):
        tests = inventory(60)
        times = {test["nodeid"]: index % 10 + 1 for index, test in enumerate(tests)}
        expected = make_plan(tests, times)
        random.Random(7).shuffle(tests)
        self.assertEqual(make_plan(tests, dict(reversed(list(times.items())))), expected)

    def test_growth_adds_runners_without_manual_configuration(self):
        self.assertGreater(len(make_plan(inventory(200), {})["shards"]),
                           len(make_plan(inventory(100), {})["shards"]))

    def test_new_tests_receive_a_nonzero_conservative_weight(self):
        tests = inventory(25)
        self.assertGreater(len(make_plan(tests, {})["shards"]),
                           len(make_plan(tests, {test["nodeid"]: 0.1 for test in tests})["shards"]))

    def test_duration_balancing_handles_a_heavy_tail(self):
        tests = inventory(50)
        times = {test["nodeid"]: 0.1 for test in tests}
        times[tests[0]["nodeid"]] = 30
        plan = make_plan(tests, times)
        self.assertTrue(all(shard["estimated_seconds"] <= 40 for shard in plan["shards"]))

    def test_empty_duplicate_and_invalid_duration_inputs_fail_closed(self):
        for tests, times in [([], {}), (inventory(1) * 2, {}),
                             *[(inventory(1), {inventory(1)[0]["nodeid"]: value})
                               for value in (0, -1, float("nan"), float("inf"), "slow")]]:
            with self.subTest(tests=tests, times=times), self.assertRaises(ValueError):
                make_plan(tests, times)

    def test_oversized_serial_group_is_not_hidden_by_more_runners(self):
        tests = inventory(10)
        for test in tests:
            test["group"] = "serial"
        with self.assertRaisesRegex(ValueError, "serial group"):
            make_plan(tests, {})

    def test_matrix_limit_is_an_error_not_silently_throttled_capacity(self):
        with self.assertRaisesRegex(ValueError, "matrix limit"):
            make_plan(inventory(257), {}, target=7, workers=1)

    def test_stale_or_incomplete_plan_is_rejected(self):
        tests = inventory(25)
        plan = make_plan(tests, {})
        with self.assertRaisesRegex(ValueError, "collected tests differ"):
            validate_plan(plan, inventory(26))
        plan["shards"][0]["nodeids"].pop()
        with self.assertRaisesRegex(ValueError, "every collected test"):
            validate_plan(plan, tests)

    def test_successful_reports_supply_all_measured_durations(self):
        tests = inventory(50)
        plan = make_plan(tests, {})
        times = verify_reports(plan, passing_reports(plan))
        self.assertEqual(times, {test["nodeid"]: 2 for test in tests})

    def test_missing_duplicate_stale_and_incomplete_reports_fail(self):
        plan = make_plan(inventory(50), {})
        reports = passing_reports(plan)
        candidates = [reports[:-1], reports + [reports[0]]]
        stale = deepcopy(reports)
        stale[0]["inventory"] = "stale"
        candidates.append(stale)
        missing = deepcopy(reports)
        missing[0]["tests"].pop(next(iter(missing[0]["tests"])))
        candidates.append(missing)
        failed = deepcopy(reports)
        failed[0]["exitstatus"] = 1
        candidates.append(failed)
        for candidate in candidates:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                verify_reports(plan, candidate)

    def test_skip_xfail_setup_and_teardown_failures_are_not_passes(self):
        plan = make_plan(inventory(1), {})
        for phase in ("setup", "call", "teardown"):
            for outcome in ("skipped", "failed"):
                reports = passing_reports(plan)
                next(iter(reports[0]["tests"].values()))["outcomes"][phase] = outcome
                with self.subTest(phase=phase, outcome=outcome), self.assertRaises(ValueError):
                    verify_reports(plan, reports)


class BundleTests(unittest.TestCase):
    def test_bundle_is_bound_to_revision_image_inputs_and_file_contents(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("Cargo.toml", "Cargo.lock", "build.rs"):
                (root / name).write_text("source")
            suite = root / "tests/e2e"
            suite.mkdir(parents=True)
            (suite / "Dockerfile").write_text("FROM pinned\n")
            requirements = suite / "requirements.txt"
            requirements.write_text("pytest==9.1.1\n")
            bundle = root / "bundle"
            bundle.mkdir()
            (bundle / "strata").write_bytes(b"binary")
            (bundle / "plan.json").write_text("{}")
            create(bundle, "revision", root)
            verify(bundle, "revision", root)
            with self.assertRaisesRegex(ValueError, "revision"):
                verify(bundle, "another", root)
            for name in ("strata", "plan.json"):
                original = (bundle / name).read_bytes()
                (bundle / name).write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "checksum"):
                    verify(bundle, "revision", root)
                (bundle / name).write_bytes(original)
            (root / "build.rs").write_text("local edit")
            with self.assertRaisesRegex(ValueError, "source differs"):
                verify(bundle, "revision", root)
            (root / "build.rs").write_text("source")
            old_key = image_key(root)
            requirements.write_text("pytest==new\n")
            self.assertNotEqual(old_key, image_key(root))
            with self.assertRaisesRegex(ValueError, "inputs"):
                verify(bundle, "revision", root)


class CliTests(unittest.TestCase):
    def test_matrix_verification_and_duration_refresh_work_outside_github(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = make_plan(inventory(25), {})
            plan_path = root / "plan.json"
            plan_path.write_text(json.dumps(plan))
            for report in passing_reports(plan):
                (root / f"shard-{report['shard']}.json").write_text(json.dumps(report))
            for command in ("matrix", "verify", "durations"):
                argv = ["e2e_ci.py", command, str(plan_path)]
                if command != "matrix":
                    argv.append(str(root))
                with self.subTest(command=command), patch.dict("os.environ", {}, clear=True), \
                     patch.object(sys, "argv", argv), patch("sys.stdout", new_callable=io.StringIO) as out:
                    ci_main()
                    result = out.getvalue()
                if command == "matrix":
                    self.assertEqual(json.loads(result.removeprefix("matrix=")),
                                     {"shard": [shard["index"] for shard in plan["shards"]]})
                elif command == "verify":
                    self.assertIn("All 25 tests passed exactly once", result)
                else:
                    self.assertEqual(len(json.loads(result)), 25)


class BudgetTests(unittest.TestCase):
    def test_budget_includes_dependency_setup_transfers_and_downstream_queues(self):
        jobs = [
            {"name": "E2E build and plan", "started_at": "2026-09-08T00:00:00Z",
             "completed_at": "2026-09-08T00:01:00Z"},
            {"name": "E2E shard 0", "started_at": "2026-09-08T00:01:15Z",
             "completed_at": "2026-09-08T00:02:00Z"},
            {"name": "E2E shard 1", "started_at": "2026-09-08T00:01:30Z",
             "completed_at": "2026-09-08T00:02:10Z"},
        ]
        elapsed, summary = critical_path(jobs, datetime(2026, 9, 8, 0, 2, 30, tzinfo=timezone.utc))
        self.assertEqual(elapsed, 150)
        self.assertIn("2 runners", summary)
        self.assertIn("| E2E build and plan | 60.0 |", summary)
        self.assertIn("| E2E shard 1 | 40.0 |", summary)

    def test_budget_reserves_teardown_time_and_always_writes_the_measurement(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.md"
            with patch.dict("os.environ", {"GITHUB_STEP_SUMMARY": str(summary)}), \
                 patch("e2e_ci.workflow_jobs", return_value=[]), \
                 patch("e2e_ci.critical_path", return_value=(174.9, "measured runtime\n")), \
                 patch("builtins.print"):
                check_budget()
                self.assertEqual(summary.read_text(), "measured runtime\n")
            with patch.dict("os.environ", {"GITHUB_STEP_SUMMARY": str(summary)}), \
                 patch("e2e_ci.workflow_jobs", return_value=[]), \
                 patch("e2e_ci.critical_path", return_value=(175, "too slow\n")), \
                 patch("builtins.print"), self.assertRaisesRegex(ValueError, "budget"):
                check_budget()
            self.assertIn("too slow", summary.read_text())

    def test_job_measurement_paginates_and_uses_the_current_run_attempt(self):
        batches = [{"jobs": [{"name": f"job-{index}"} for index in range(100)]},
                   {"jobs": [{"name": "last"}]}]
        responses = [io.BytesIO(json.dumps(batch).encode()) for batch in batches]
        with patch.dict("os.environ", {"GITHUB_REPOSITORY": "example/strata", "GITHUB_RUN_ID": "123",
                                       "GITHUB_RUN_ATTEMPT": "2", "GH_TOKEN": "test-only",
                                       "GITHUB_API_URL": "https://api.github.com"}), \
             patch("e2e_ci.urlopen", side_effect=responses) as request:
            self.assertEqual(len(workflow_jobs()), 101)
            self.assertIn("/runs/123/attempts/2/jobs?per_page=100&page=2",
                          request.call_args.args[0].full_url)

    def test_missing_job_timestamps_cannot_be_reported_as_a_fast_pass(self):
        with self.assertRaises(ValueError):
            critical_path([], datetime.now(timezone.utc))


if __name__ == "__main__":
    unittest.main()
