#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""CI matrix, exact-coverage gate, measured critical path, and duration refresh."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import sys
from urllib.request import Request, urlopen

from e2e_diagnostics import require_dependencies

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests/e2e"))
from harness.sharding import validate_plan, verify_reports  # noqa: E402

TARGET_SECONDS = 180


def critical_path(jobs: list[dict], now: datetime) -> tuple[float, str]:
    build = [job for job in jobs if job["name"] == "E2E build and plan"]
    shards = [job for job in jobs if job["name"].startswith("E2E shard ")
              and job["name"].removeprefix("E2E shard ").isdigit()]
    if len(build) != 1:
        raise ValueError("cannot measure E2E critical path without exactly one build job")
    if not build[0].get("created_at"):
        raise ValueError("cannot measure initial E2E queue time without the build creation timestamp")
    parse = lambda value: datetime.fromisoformat(value.replace("Z", "+00:00"))
    start = parse(build[0]["created_at"])
    elapsed = (now - start).total_seconds()
    if elapsed < 0:
        raise ValueError("runner clock precedes the initial E2E queue timestamp")
    lines = ["## E2E critical path", "", f"**{elapsed:.1f}s elapsed; <{TARGET_SECONDS}s target (informational)**, "
             f"{len(shards)} runners (two isolated GUI workers each).", "",
             "Includes the initial E2E queue, build-job startup, dependency/cache setup, compilation, "
             "artifact transfers, downstream runner queues, tests, and aggregation through this measurement.", "",
             "| Job | Before start (s) | Run (s) |", "| --- | ---: | ---: |"]
    for job in build + sorted(shards, key=lambda job: job["name"]):
        started = parse(job["started_at"])
        queued = (started - parse(job["created_at"])).total_seconds()
        end = parse(job["completed_at"]) if job.get("completed_at") else now
        lines.append(f"| {job['name']} | {queued:.1f} | {(end - started).total_seconds():.1f} |")
    if build[0].get("conclusion") in {"failure", "cancelled", "timed_out"}:
        lines += ["", "The build/plan prerequisite failed. This duration includes failed bootstrap; "
                  "it is not a measurement of GUI scenario execution. See the E2E failure summary."]
    return elapsed, "\n".join(lines) + "\n"


def publish_summary(summary):
    print(summary)
    if path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(path).open("a") as stream:
            stream.write(summary)


def report_timing():
    try:
        jobs = workflow_jobs()
        elapsed, summary = critical_path(jobs, datetime.now(timezone.utc))
    except (OSError, ValueError, KeyError, TypeError):
        publish_summary("## E2E timing\n\nTiming data is unavailable; test and coverage results are unaffected.\n")
        return
    publish_summary(summary)
    if elapsed >= TARGET_SECONDS:
        print("::notice title=E2E timing::The three-minute performance target was exceeded. "
              "Timing is informational and does not fail passing tests.")


def workflow_jobs() -> list[dict]:
    api = os.environ.get("GITHUB_API_URL", "https://api.github.com")
    path = (f"{api}/repos/{os.environ['GITHUB_REPOSITORY']}/actions/runs/"
            f"{os.environ['GITHUB_RUN_ID']}/attempts/{os.environ['GITHUB_RUN_ATTEMPT']}/jobs")
    jobs = []
    for page in range(1, 20):
        request = Request(f"{path}?per_page=100&page={page}", headers={
            "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        })
        with urlopen(request, timeout=20) as response:
            batch = json.load(response)["jobs"]
        jobs.extend(batch)
        if len(batch) < 100:
            return jobs
    raise ValueError("too many workflow jobs to measure")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("timing")
    sub.add_parser("dependencies")
    matrix = sub.add_parser("matrix")
    matrix.add_argument("plan", type=Path)
    for command in ("verify", "durations"):
        child = sub.add_parser(command)
        child.add_argument("plan", type=Path)
        child.add_argument("reports", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "dependencies":
            require_dependencies(workflow_jobs, publish_summary)
            return
        if args.command == "timing":
            report_timing()
            return
        plan = json.loads(args.plan.read_text())
        validate_plan(plan)
        if args.command == "matrix":
            print("matrix=" + json.dumps({"shard": [shard["index"] for shard in plan["shards"]]},
                                         separators=(",", ":")))
            return
        reports = [json.loads(path.read_text()) for path in sorted(args.reports.glob("shard-*.json"))]
        durations = verify_reports(plan, reports)
        if args.command == "durations":
            print(json.dumps({key: round(value, 3) for key, value in durations.items()}, indent=2))
            return
        publish_summary(f"All {len(durations)} tests passed exactly once; no skipped tests.\n")
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"E2E gate: {error}\n")


if __name__ == "__main__":
    main()
