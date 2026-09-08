# SPDX-License-Identifier: GPL-3.0-or-later
"""Deterministic duration-balanced runner plans, independent of GTK and pytest."""

from __future__ import annotations

import hashlib
import json
import math

SCHEMA = 1
UNKNOWN_SECONDS = 5.0
TARGET_SECONDS = 30.0
WORKERS = 2
MAX_SHARDS = 256


def inventory_digest(tests: list[dict]) -> str:
    return hashlib.sha256(json.dumps(sorted(tests, key=lambda test: test["nodeid"]),
                                     sort_keys=True).encode()).hexdigest()


def make_plan(tests: list[dict], durations: dict[str, float], *,
              target: float = TARGET_SECONDS, workers: int = WORKERS) -> dict:
    if not tests or workers < 1 or not math.isfinite(target) or target <= 0:
        raise ValueError("a plan needs tests, positive workers, and a positive time budget")
    if len({test["nodeid"] for test in tests}) != len(tests):
        raise ValueError("duplicate collected node IDs")
    groups: dict[str, dict] = {}
    for test in sorted(tests, key=lambda test: test["nodeid"]):
        nodeid = test["nodeid"]
        seconds = durations.get(nodeid, UNKNOWN_SECONDS)
        if not isinstance(seconds, (int, float)) or not math.isfinite(seconds) or seconds <= 0:
            raise ValueError(f"invalid duration for {nodeid}: {seconds!r}")
        # Allow for host variance, and never make tiny tests free to schedule.
        estimate = max(0.1, seconds) * 1.25
        key = test.get("group") or nodeid
        group = groups.setdefault(key, {"nodeids": [], "seconds": 0.0})
        group["nodeids"].append(nodeid)
        group["seconds"] += estimate
    ordered = sorted(groups.values(), key=lambda group: (-group["seconds"], group["nodeids"]))
    if ordered[0]["seconds"] > target:
        raise ValueError(f"one serial group exceeds {target:g}s; split the scenario/group: "
                         f"{ordered[0]['nodeids']}")

    minimum = max(1, math.ceil(sum(group["seconds"] for group in ordered) / (target * workers)))
    for count in range(minimum, min(len(ordered), MAX_SHARDS) + 1):
        lanes = [[0.0] * workers for _ in range(count)]
        shards = [[] for _ in range(count)]
        for group in ordered:
            shard, lane = min(((s, w) for s in range(count) for w in range(workers)),
                              key=lambda pair: (lanes[pair[0]][pair[1]], pair))
            lanes[shard][lane] += group["seconds"]
            shards[shard].extend(group["nodeids"])
        if max(max(lane) for lane in lanes) <= target:
            return {"schema": SCHEMA, "inventory": inventory_digest(tests),
                    "workers": workers, "target_seconds": target,
                    "shards": [{"index": index, "nodeids": nodeids,
                                "estimated_seconds": round(max(lanes[index]), 2)}
                               for index, nodeids in enumerate(shards) if nodeids]}
    raise ValueError(f"suite exceeds the {MAX_SHARDS}-runner matrix limit")


def validate_plan(plan: dict, tests: list[dict] | None = None) -> None:
    if plan.get("schema") != SCHEMA or not plan.get("shards"):
        raise ValueError("unsupported or empty shard plan")
    nodeids = []
    for index, shard in enumerate(plan["shards"]):
        if shard["index"] != index or not shard["nodeids"]:
            raise ValueError("non-contiguous or empty shard")
        nodeids.extend(shard["nodeids"])
    if len(nodeids) != len(set(nodeids)):
        raise ValueError("a test is assigned to multiple shards")
    if tests is not None:
        if plan["inventory"] != inventory_digest(tests):
            raise ValueError("collected tests differ from the shard plan; rebuild the bundle")
        if set(nodeids) != {test["nodeid"] for test in tests}:
            raise ValueError("plan does not cover every collected test exactly once")


def verify_reports(plan: dict, reports: list[dict]) -> dict[str, float]:
    validate_plan(plan)
    by_shard = {}
    for report in reports:
        index = report["shard"]
        if index in by_shard or not isinstance(index, int) or not 0 <= index < len(plan["shards"]):
            raise ValueError("duplicate or unexpected shard report")
        by_shard[index] = report
    if set(by_shard) != set(range(len(plan["shards"]))):
        raise ValueError("missing shard reports")
    durations = {}
    for shard in plan["shards"]:
        report = by_shard[shard["index"]]
        if report["inventory"] != plan["inventory"] or report["exitstatus"] != 0:
            raise ValueError(f"shard {shard['index']} failed or used a stale inventory")
        results = report["tests"]
        if set(results) != set(shard["nodeids"]):
            raise ValueError(f"shard {shard['index']} did not execute its exact assignment")
        for nodeid, result in results.items():
            if result["outcomes"] != {"setup": "passed", "call": "passed", "teardown": "passed"}:
                raise ValueError(f"test did not pass all phases (skips are not passes): {nodeid}")
            seconds = result["seconds"]
            if not math.isfinite(seconds) or seconds < 0:
                raise ValueError(f"invalid measured duration: {nodeid}")
            durations[nodeid] = max(0.1, seconds)
    return dict(sorted(durations.items()))
