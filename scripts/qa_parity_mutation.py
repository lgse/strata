#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Proves the Rune target's parity runs can fail: each patch under tests/qa/parity/mutations
breaks one behaviour of strata-app, and the scenario it is named for must catch it.

Checked the way scripts/qa_mutation.py checks the GTK application's patches: the scenario's
required runs pass on the Rune target unpatched, and at least one fails a step with the patch
applied. A patch is `<scenario>.patch`.
"""

from __future__ import annotations

import os
import subprocess
import sys
import time
from pathlib import Path

import qa_parity
from qa_mutation import (
    Mutation, Report, Scenario, baseline_verdict, mutated_verdict, own_runs, parse_qa_log,
    write_summary,
)

REPOSITORY = Path(__file__).resolve().parents[1]
MUTATIONS = REPOSITORY / "tests/qa/parity/mutations"
OUTPUT = REPOSITORY / "target/qa-parity-mutation"


def patches(directory: Path | None = None) -> list[str]:
    """The scenarios the patches are named for."""

    return sorted(path.stem for path in (directory or MUTATIONS).glob("*.patch"))


def unguarded(scenarios: list[str], ledger: dict) -> list[str]:
    """The patches whose scenario has no required run: nothing would be run to catch them."""

    return [scenario for scenario in scenarios
            if not qa_parity.selected(ledger, "slice", "required", scenario)]


def run_rune(scenario: str, destination: Path) -> tuple[dict[str, str], int]:
    """The scenario's required runs on the Rune target, in the pinned image."""

    destination.mkdir(parents=True, exist_ok=True)
    log = destination / "qa-parity.log"
    started = time.monotonic()
    command = [str(REPOSITORY / "scripts/qa-parity.sh"), "slice", "--target", "rune",
               "--scenario", scenario]
    with log.open("w", encoding="utf-8") as output:
        status = subprocess.run(
            command, cwd=REPOSITORY, stdout=output, stderr=subprocess.STDOUT, check=False,
        ).returncode
    print(f"   qa-parity.sh exited {status} after {time.monotonic() - started:.0f}s; "
          f"log {log.relative_to(REPOSITORY)}", flush=True)
    return parse_qa_log(log.read_text(encoding="utf-8", errors="replace")), status


def check() -> int:
    started = time.monotonic()
    scenarios = patches()
    (OUTPUT / "summary.md").unlink(missing_ok=True)
    if not scenarios:
        print(f"No patch under {MUTATIONS.relative_to(REPOSITORY)}.", file=sys.stderr)
        return 2
    if missing := unguarded(scenarios, qa_parity.load()):
        print(f"No required run in the ledger for: {', '.join(missing)}", file=sys.stderr)
        return 2
    reports = []
    for scenario in scenarios:
        print(f"== {scenario}: unmodified tree", flush=True)
        runs, _ = run_rune(scenario, OUTPUT / "baseline" / scenario)
        report = Report(Scenario(scenario, f"{scenario}.patch"),
                        baseline_verdict(own_runs(scenario, runs)))
        reports.append(report)
        if not report.baseline.caught:
            print(f"== {scenario}: skipped, {report.baseline.note}", flush=True)
            continue
        print(f"== {scenario}: mutated tree", flush=True)
        with Mutation(scenario, mutations=MUTATIONS):
            runs, status = run_rune(scenario, OUTPUT / "mutations" / scenario)
        report.runs = own_runs(scenario, runs)
        report.mutated = mutated_verdict(report.runs, status)
    summary = write_summary(reports, time.monotonic() - started, OUTPUT)
    print("\n" + summary)
    return 0 if all(report.caught for report in reports) else 1


def main() -> int:
    if os.environ.get("STRATA_QA_UPDATE_BASELINES") == "1":
        print("STRATA_QA_UPDATE_BASELINES must be unset for a mutated run", file=sys.stderr)
        return 2
    try:
        return check()
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(main())
