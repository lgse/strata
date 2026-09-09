# SPDX-License-Identifier: MIT
"""Collect once for CI, validate every worker's inventory, report all phases."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from .sharding import make_plan, validate_plan

_STATE = pytest.StashKey[dict]()


def canonical_nodeid(nodeid: str) -> str:
    return nodeid.removesuffix("@visual-baselines")


def pytest_addoption(parser):
    group = parser.getgroup("e2e-ci")
    group.addoption("--e2e-write-plan", type=Path)
    group.addoption("--e2e-durations", type=Path, default=Path("tests/e2e/durations.json"))
    group.addoption("--e2e-plan", type=Path)
    group.addoption("--e2e-shard", type=int)
    group.addoption("--e2e-report", type=Path)


def pytest_configure(config):
    plan_path = config.getoption("e2e_plan")
    shard = config.getoption("e2e_shard")
    if (plan_path is None) != (shard is None):
        raise pytest.UsageError("--e2e-plan and --e2e-shard must be supplied together")
    state = {"tests": {}, "plan": None, "shard": shard, "collection_skips": []}
    if plan_path:
        try:
            plan = json.loads(plan_path.read_text())
            validate_plan(plan)
            if not 0 <= shard < len(plan["shards"]):
                raise ValueError("shard index out of range")
            state["plan"] = plan
        except (OSError, ValueError, KeyError, TypeError) as error:
            raise pytest.UsageError(str(error)) from error
    if config.getoption("e2e_write_plan") and not config.getoption("collectonly"):
        raise pytest.UsageError("--e2e-write-plan requires --collect-only")
    config.stash[_STATE] = state


@pytest.hookimpl(trylast=True)
def pytest_collection_modifyitems(config, items):
    tests = [{"nodeid": canonical_nodeid(item.nodeid),
              "group": "visual-baselines" if item.get_closest_marker("baseline") else None}
             for item in items]
    state = config.stash[_STATE]
    try:
        if (config.getoption("e2e_write_plan") or state["plan"]) and state["collection_skips"]:
            raise ValueError(f"CI collection skipped modules: {state['collection_skips']}")
        if path := config.getoption("e2e_write_plan"):
            durations_path = config.getoption("e2e_durations")
            durations = json.loads(durations_path.read_text()) if durations_path.exists() else {}
            plan = make_plan(tests, durations)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(plan, indent=2) + "\n")
        if plan := state["plan"]:
            validate_plan(plan, tests)
            assignment = plan["shards"][state["shard"]]["nodeids"]
            assigned = set(assignment)
            selected, deselected = [], []
            for item in items:
                (selected if canonical_nodeid(item.nodeid) in assigned else deselected).append(item)
            order = {nodeid: index for index, nodeid in enumerate(assignment)}
            items[:] = sorted(selected, key=lambda item: order[canonical_nodeid(item.nodeid)])
            config.hook.pytest_deselected(items=deselected)
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise pytest.UsageError(str(error)) from error


class ReportCollector:
    def __init__(self, config):
        self.config = config

    def pytest_collectreport(self, report):
        if report.skipped:
            self.config.stash[_STATE]["collection_skips"].append(report.nodeid)

    def pytest_runtest_logreport(self, report):
        state = self.config.stash[_STATE]
        result = state["tests"].setdefault(canonical_nodeid(report.nodeid),
                                           {"outcomes": {}, "seconds": 0.0})
        if report.when in result["outcomes"]:
            outcome = "duplicate"
        elif hasattr(report, "wasxfail"):
            outcome = "xfail"
        else:
            outcome = report.outcome
        result["outcomes"][report.when] = outcome
        result["seconds"] += report.duration


def pytest_sessionstart(session):
    session.config.pluginmanager.register(ReportCollector(session.config), "e2e-report-collector")


def pytest_sessionfinish(session, exitstatus):
    config = session.config
    if hasattr(config, "workerinput") or not (path := config.getoption("e2e_report")):
        return
    state = config.stash[_STATE]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"shard": state["shard"], "exitstatus": int(exitstatus),
                               "inventory": state["plan"]["inventory"] if state["plan"] else None,
                               "tests": state["tests"]}, indent=2) + "\n")
