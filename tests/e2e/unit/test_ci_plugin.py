# SPDX-License-Identifier: GPL-3.0-or-later

import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from harness.sharding import verify_reports


@pytest.fixture
def plugin_suite(tmp_path):
    (tmp_path / "pytest.ini").write_text("[pytest]\nmarkers = baseline: visual fixture\n")
    (tmp_path / "test_sample.py").write_text(
        "import pytest\n"
        "@pytest.mark.parametrize('value', range(15))\n"
        "def test_parameter(value):\n"
        "    assert value >= 0\n"
        "@pytest.mark.baseline\n"
        "@pytest.mark.xdist_group('visual-baselines')\n"
        "def test_visual():\n"
        "    pass\n"
    )
    return tmp_path


def run_pytest(root, *args):
    env = {**os.environ, "PYTHONPATH": str(Path(__file__).resolve().parents[1]),
           "PYTEST_DISABLE_PLUGIN_AUTOLOAD": "1"}
    return subprocess.run([sys.executable, "-m", "pytest", "-p", "xdist.plugin",
                           "-p", "harness.ci_plugin", "--rootdir", str(root), *args],
                          cwd=root, env=env, capture_output=True, text=True, timeout=30)


def collect_plan(root):
    path = root / "plan.json"
    result = run_pytest(root, "--collect-only", "-q", f"--e2e-write-plan={path}")
    assert result.returncode == 0, result.stdout + result.stderr
    return path, json.loads(path.read_text())


def test_real_collection_and_xdist_execution_cover_every_parameter_once(plugin_suite):
    path, plan = collect_plan(plugin_suite)
    reports = []
    for shard in plan["shards"]:
        report_path = plugin_suite / f"shard-{shard['index']}.json"
        result = run_pytest(plugin_suite, "-n", "2", "--dist=loadgroup",
                            f"--e2e-plan={path}", f"--e2e-shard={shard['index']}",
                            f"--e2e-report={report_path}")
        assert result.returncode == 0, result.stdout + result.stderr
        reports.append(json.loads(report_path.read_text()))
    assert len(verify_reports(plan, reports)) == 16


def test_changed_collection_and_invalid_shard_fail_before_execution(plugin_suite):
    path, _ = collect_plan(plugin_suite)
    invalid = run_pytest(plugin_suite, f"--e2e-plan={path}", "--e2e-shard=999")
    assert invalid.returncode != 0
    assert "out of range" in invalid.stderr
    with (plugin_suite / "test_sample.py").open("a") as stream:
        stream.write("\ndef test_added():\n    pass\n")
    result = run_pytest(plugin_suite, f"--e2e-plan={path}", "--e2e-shard=0")
    assert result.returncode != 0
    assert "collected tests differ" in result.stderr


@pytest.mark.parametrize("body", ["pytest.skip('not a pass')", "assert False"])
def test_skips_and_assertion_failures_cannot_satisfy_coverage(plugin_suite, body):
    (plugin_suite / "test_sample.py").write_text(f"import pytest\ndef test_case():\n    {body}\n")
    path, plan = collect_plan(plugin_suite)
    report = plugin_suite / "shard-0.json"
    run_pytest(plugin_suite, f"--e2e-plan={path}", "--e2e-shard=0", f"--e2e-report={report}")
    with pytest.raises(ValueError):
        verify_reports(plan, [json.loads(report.read_text())])


def test_collection_time_skip_cannot_remove_a_module_from_the_plan(plugin_suite):
    (plugin_suite / "test_skipped.py").write_text(
        "import pytest\npytest.skip('not available', allow_module_level=True)\n"
        "def test_lost():\n    pass\n"
    )
    result = run_pytest(plugin_suite, "--collect-only", "--e2e-write-plan=plan.json")
    assert result.returncode != 0
    assert "collection skipped modules" in result.stderr
    assert not (plugin_suite / "plan.json").exists()


def test_shard_options_are_paired_and_plan_writing_is_collection_only(plugin_suite):
    for args in [("--e2e-shard=0",), ("--e2e-plan=absent.json",), ("--e2e-write-plan=plan.json",)]:
        result = run_pytest(plugin_suite, *args)
        assert result.returncode == 4, result.stdout + result.stderr
