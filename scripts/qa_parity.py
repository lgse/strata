#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reads the parity ledger for scripts/qa-parity.sh: the runs a selection names."""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]
LEDGER = REPOSITORY / "tests/qa/parity/slice.toml"
GATES = {"g1": ("G1",), "g2": ("G2",), "g3": ("G3",), "slice": ("G1", "G2", "G3")}
STATUSES = ("required", "pending")


def selected(ledger: dict, selection: str, status: str, scenario: str | None = None) -> list[str]:
    """The `<scenario>/<run>` names of a selection's runs with the status, in ledger order.

    With `scenario`, only that scenario's runs.
    """

    gates = GATES[selection]
    return [f"{run['scenario']}/{run['run']}" for run in ledger.get("run", [])
            if run["gate"] in gates and run["status"] == status
            and scenario in (None, run["scenario"])]


def load(ledger: Path = LEDGER) -> dict:
    with ledger.open("rb") as stream:
        return tomllib.load(stream)


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("selection", choices=sorted(GATES))
    parser.add_argument("--status", choices=STATUSES, default="required")
    parser.add_argument("--scenario", help="only this scenario's runs")
    parser.add_argument("--ledger", type=Path, default=LEDGER)
    args = parser.parse_args(arguments)
    for name in selected(load(args.ledger), args.selection, args.status, args.scenario):
        print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
