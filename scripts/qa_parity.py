#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reads the parity ledger for scripts/qa-parity.sh: the runs a selection names, and the
failures its allowances let `slice` look at again."""

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
# The runner's line for a failed test, tests/qa/harness.rs.
FAILED = re.compile(r"^test (\S+) \.\.\. FAILED\b")


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


def excused(ledger: dict, log: str) -> tuple[list[str], list[str]]:
    """A failed whole-suite log, sorted: the flaky runs to run again alone, and the rest.

    The rest fails `slice` whatever a rerun would do. A log that names no failed test is not
    excused either: the suite failed some other way, and the caller treats that as failure.
    """

    allowed = flaky(ledger)
    failed = failed_tests(log)
    return ([name for name in failed if name in allowed],
            [name for name in failed if name not in allowed])


def report_excused(ledger: dict, log: Path) -> int:
    """Prints the flaky runs to run again; exits 1 when the failure is not theirs alone."""

    again, others = excused(ledger, log.read_text(encoding="utf-8", errors="replace"))
    if others:
        print("Failed in the whole GTK suite, and no allowance names them: " + ", ".join(others),
              file=sys.stderr)
        return 1
    if not again:
        print("The whole GTK suite failed without naming a failed test.", file=sys.stderr)
        return 1
    for name in again:
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
                             "log of the whole GTK suite, or exit 1 if anything else failed")
    args = parser.parse_args(arguments)
    if args.failed_in:
        return report_excused(load(args.ledger), args.failed_in)
    for name in selected(load(args.ledger), args.selection, args.status, args.scenario):
        print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
