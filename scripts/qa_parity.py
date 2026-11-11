#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reads the parity ledger for scripts/qa-parity.sh: the runs a selection names, and the
failures of the whole GTK suite its allowances let `slice` pass over."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]
LEDGER = REPOSITORY / "tests/qa/parity/slice.toml"
GATES = {"g1": ("G1",), "g2": ("G2",), "g3": ("G3",), "slice": ("G1", "G2", "G3")}
STATUSES = ("required", "pending")
# The runner's line for a failed test and its closing count of them, tests/qa/harness.rs.
FAILED = re.compile(r"^test (\S+) \.\.\. FAILED\b")
RESULT = re.compile(r"^test result: FAILED\. \d+ passed; (\d+) failed;")


def selected(ledger: dict, selection: str, status: str, scenario: str | None = None) -> list[str]:
    """The `<scenario>/<run>` names of a selection's runs with the status, in ledger order.

    With `scenario`, only that scenario's runs.
    """

    gates = GATES[selection]
    return [f"{run['scenario']}/{run['run']}" for run in ledger.get("run", [])
            if run["gate"] in gates and run["status"] == status
            and scenario in (None, run["scenario"])]


def flaky(ledger: dict) -> list[str]:
    """The `<scenario>/<run>` names the ledger allows to fail in the whole GTK suite."""

    return [f"{run['scenario']}/{run['run']}" for run in ledger.get("flaky", [])]


def stale_patches(ledger: dict) -> list[str]:
    """The GTK mutation patches the ledger says stopped applying before the slice."""

    return [entry["patch"] for entry in ledger.get("stale_patch", [])]


def failed_tests(log: str) -> list[str]:
    """The tests a run of the runner reports as failed, in the order they failed."""

    return [match[1] for line in log.splitlines() if (match := FAILED.match(line.rstrip()))]


def counted_failures(log: str) -> int | None:
    """How many tests the runner's closing line says failed; none if it never got there."""

    counts = [int(match[1]) for line in log.splitlines() if (match := RESULT.match(line.rstrip()))]
    return counts[-1] if counts else None


def excused(ledger: dict, log: str) -> tuple[list[str], list[str]]:
    """A failed whole-suite log, sorted: the flaky runs that failed, and every other test.

    The flaky runs do not fail `slice`; anything else does.
    """

    allowed = flaky(ledger)
    failed = failed_tests(log)
    return ([name for name in failed if name in allowed],
            [name for name in failed if name not in allowed])


def refusal(ledger: dict, log: str) -> str | None:
    """Why a failed whole-suite log fails `slice`, or nothing when the allowance covers it.

    It covers a suite that ran to its end and failed for the ledger's flaky runs alone. A
    suite that names no failed test, or whose closing count is not the tests it named, failed
    some other way: a build error, or a runner that stopped part way.
    """

    passed_over, others = excused(ledger, log)
    if others:
        return f"{', '.join(others)} failed in the whole GTK suite, and no allowance names them."
    if not passed_over:
        return "the whole GTK suite failed without naming a failed test."
    if counted_failures(log) != len(passed_over):
        return "the whole GTK suite did not end with a count of the failures it named."
    return None


def report_excused(ledger: dict, log: Path) -> int:
    """Prints the flaky runs that failed; exits 1 when the failure is not theirs alone."""

    text = log.read_text(encoding="utf-8", errors="replace")
    if reason := refusal(ledger, text):
        print(f"NO ALLOWANCE: {reason}", file=sys.stderr)
        return 1
    for name in excused(ledger, text)[0]:
        print(name)
    return 0


def load(ledger: Path = LEDGER) -> dict:
    with ledger.open("rb") as stream:
        return tomllib.load(stream)


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("selection", choices=sorted(GATES))
    parser.add_argument("--status", choices=STATUSES, default="required")
    parser.add_argument("--scenario", help="only this scenario's runs")
    parser.add_argument("--ledger", type=Path, default=LEDGER)
    parser.add_argument("--failed-in", type=Path, metavar="LOG",
                        help="instead of a selection's runs: the flaky runs that failed in this "
                             "log of the whole GTK suite, or exit 1 if it failed any other way")
    args = parser.parse_args(arguments)
    if args.failed_in:
        return report_excused(load(args.ledger), args.failed_in)
    for name in selected(load(args.ledger), args.selection, args.status, args.scenario):
        print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
