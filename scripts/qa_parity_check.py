#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Checks the parity ledger before scripts/qa-parity.sh selects from it: everything the
runner's own reader refuses, tests/qa/parity.rs, this one refuses too.

Both readers are held to one table, tests/qa/parity_cases.toml, which the runner's tests and
the scripts' tests read: a problem is reported in the same words by both. The runs the
runner plans are worked out here from the scenario headers, as tests/qa/variables.rs does.
"""

from __future__ import annotations

import json
import tomllib
from pathlib import Path

STATUSES = ("required", "pending", "excluded")
HOSTS = ("offscreen", "session")
REASONS = ("harness", "component", "rune", "platform", "toolkit", "deviation", "scope")
GATES = ("G1", "G2", "G3")
# Every field an entry may have; the first four are on every entry.
MANDATORY = ("scenario", "run", "package", "status")
FIELDS = (*MANDATORY, "gate", "host", "reason", "blocker", "ruling")
VALUES = {"status": STATUSES, "host": HOSTS, "reason": REASONS, "gate": GATES}
# The parity plan's packages by track letter, and the slice's with the gate each owns.
TRACKS = {"H": range(0, 6), "B": range(1, 4), "F": range(1, 6), "A": range(1, 22), "E": range(1, 5)}
SLICE = {"V7": "G1", "V8": "G2", "V9": "G3"}
# The slice's set: the Columns runs of these folders have a gate, and G1 is one run of them.
SET = {"keyboard_navigation": "G2", "selection": "G2", "escape_selection": "G2",
       "background_selection": "G2", "hidden_files": "G2", "startup_arguments": "G2",
       "inline_renaming": "G3"}
FIRST_LIGHT = ("arrow-keys-move-focus", "columns-arrows")
COLUMNS = "columns"
# What to do about a run the ledger does not list; HELP in tests/qa/parity.rs.
HELP = ("A run that is not listed goes into tests/qa/parity/<its folder>.toml, at its place in "
        "the runner's order, as pending on scope under its package: docs/testing.md, "
        "\"Who owns a run\", says which.")


def runs(header: dict) -> list[tuple[str, str]]:
    """A scenario's runs as `(mode, label)`, in the runner's order: `runs` in variables.rs."""

    variants = header.get("variants") or [None]
    planned = []
    for variant in variants:
        modes = header["modes"] if variant is None or variant.get("modes") is None else variant["modes"]
        planned += [(mode, mode if variant is None else f"{mode}-{variant['id']}") for mode in modes]
    return planned


def gate(folder: str, scenario: str, mode: str, label: str) -> str | None:
    """The gate of a run: G1's one run, or its folder's when it is a Columns run of the set."""

    if mode != COLUMNS:
        return None
    return "G1" if (scenario, label) == FIRST_LIGHT else SET.get(folder)


def header(path: Path) -> dict:
    lines = path.read_text(encoding="utf-8").splitlines()
    return json.loads(next(line for line in lines if line.strip()))


def planned(scenarios: Path) -> list[dict]:
    """Every run of every scenario file, as the runner plans them and in its order."""

    files = sorted(path for folder in scenarios.iterdir() if folder.is_dir()
                   for path in folder.glob("*.jsonl"))
    return [{"folder": path.parent.name, "scenario": path.stem, "run": label,
             "gate": gate(path.parent.name, path.stem, mode, label)}
            for path in files for mode, label in runs(header(path))]


def name(entry: dict) -> str:
    return f"{entry.get('scenario', '?')}/{entry.get('run', '?')}"


def unknown(entry: dict) -> list[str]:
    return [key for key in entry if key not in FIELDS]


def missing(entry: dict) -> list[str]:
    return [key for key in MANDATORY if key not in entry]


def untyped(entry: dict) -> list[str]:
    return [key for key in FIELDS if not isinstance(entry.get(key, ""), str)]


def unvalued(entry: dict) -> list[str]:
    return [key for key, values in VALUES.items() if entry.get(key, values[0]) not in values]


# What keeps an entry from being read at all, and how it is said, first fault first.
UNREADABLE = ((unknown, "has the unknown field {key}"), (missing, "has no {key}"),
              (untyped, "has a {key} that is no text"), (unvalued, "has the unknown {key} {value!r}"))


def unreadable(entry: object) -> str | None:
    """What keeps an entry from being read at all: a field or a value the ledger lacks."""

    if not isinstance(entry, dict):
        return "an entry is no table"
    for find, problem in UNREADABLE:
        if found := find(entry):
            return f"{name(entry)} " + problem.format(key=found[0], value=entry.get(found[0]))
    return None


def parse(folder: str, text: str) -> tuple[list[dict], str | None]:
    """A file's entries, or why it cannot be read."""

    try:
        tables = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        return [], f"{folder}.toml: {error}"
    stray = [key for key in tables if key != "run"]
    if stray or not isinstance(tables.get("run", []), list):
        return [], f"{folder}.toml: only [[run]] entries belong here, not {(stray or ['run'])[0]}"
    entries = tables.get("run", [])
    problems = [problem for entry in entries if (problem := unreadable(entry))]
    return entries, f"{folder}.toml: {problems[0]}" if problems else None


def is_package(package: str) -> bool:
    track, number = package[:1], package[1:]
    return track in TRACKS and number.isascii() and number.isdigit() \
        and str(int(number)) == number and int(number) in TRACKS[track]


def misplaced(entry: dict, run: dict | None) -> str | None:
    """What is wrong with where the entry stands: no such run in its folder, or a gate that
    is not the run's."""

    if run is None:
        return "is no run of the folder"
    planned_gate = run.get("gate")
    if planned_gate == entry.get("gate"):
        return None
    if planned_gate:
        return f"belongs to gate {planned_gate}"
    return "has a gate, and is no Columns run of the slice's set"


def unowned(entry: dict) -> str | None:
    """What is wrong with the owner: no package of the plan, or one of the slice's on a run
    it did not take to `required`."""

    package = entry["package"]
    quoted = json.dumps(package)
    if package in SLICE:
        kept = entry["status"] == "required" and entry.get("gate") == SLICE[package]
        return None if kept else f"names {quoted}, which owns only the required runs of gate {SLICE[package]}"
    return None if is_package(package) else f"names {quoted}, which is no package of the parity plan"


def said(entry: dict, key: str) -> bool:
    return bool(entry.get(key, "").strip())


def blocked(entry: dict) -> bool:
    return "reason" in entry or "blocker" in entry


def required_without(entry: dict) -> str | None:
    if "host" not in entry:
        return "is required without a host"
    if blocked(entry) or "ruling" in entry:
        return "is required, so it has no reason, blocker or ruling"
    return None


def pending_without(entry: dict) -> str | None:
    if "reason" not in entry:
        return "is pending without a reason"
    if not said(entry, "blocker"):
        return "is pending without a blocker"
    return "is pending, so it has no ruling" if "ruling" in entry else None


def excluded_without(entry: dict) -> str | None:
    if not said(entry, "ruling"):
        return "is excluded without a ruling"
    if "host" in entry or blocked(entry):
        return "is excluded, so it has no host, reason or blocker"
    return None


UNEXPLAINED = {"required": required_without, "pending": pending_without, "excluded": excluded_without}


def standing(entry: dict, repeated: bool, run: dict | None) -> list[str]:
    """An entry's own problems: listed twice, and the first thing wrong with it."""

    found = misplaced(entry, run) or unowned(entry) or UNEXPLAINED[entry["status"]](entry)
    twice = [f"{name(entry)} is listed twice"] if repeated else []
    return twice + ([f"{name(entry)} {found}"] if found else [])


def disordered(entry: dict, place: int | None, previous: tuple[int, str] | None) -> list[str]:
    """What is wrong with an entry that follows one the runner plans after it. `previous` is
    the last entry that named a run, and where the runner plans it."""

    if place is None or previous is None:
        return []
    late = place < previous[0]
    return [f"{name(entry)} is out of the runner's order: it belongs before {previous[1]}"] if late else []


def folder_problems(entries: list[dict], runs_of_folder: list[dict]) -> list[str]:
    """Everything wrong with one folder's file against the folder's runs, which are in the
    runner's order: `folder_problems` in tests/qa/parity.rs, line for line."""

    places = {(run["scenario"], run["run"]): place for place, run in enumerate(runs_of_folder)}
    problems, listed, previous = [], set(), None
    for entry in entries:
        key = (entry["scenario"], entry["run"])
        place = places.get(key)
        run = None if place is None else runs_of_folder[place]
        problems += standing(entry, key in listed, run) + disordered(entry, place, previous)
        previous = previous if place is None else (place, name(entry))
        listed.add(key)
    return problems + [f"{name(run)} is not listed" for run in runs_of_folder
                       if (run["scenario"], run["run"]) not in listed]


def problems(files: dict[str, str], planned_runs: list[dict]) -> list[str]:
    """Everything wrong with the ledger, one line each: `files` is each folder's file text.

    A file that cannot be read is one problem, and the rest of it is not looked at.
    """

    folders: dict[str, list[dict]] = {}
    for run in planned_runs:
        folders.setdefault(run["folder"], []).append(run)
    found = []
    for folder in sorted(folders):
        if folder not in files:
            found.append(f"{folder}.toml is missing: tests/qa/scenarios/{folder} has no ledger")
            continue
        entries, refusal = parse(folder, files[folder])
        found += [refusal] if refusal else [f"{folder}.toml: {problem}"
                                            for problem in folder_problems(entries, folders[folder])]
    for folder in sorted(set(files) - set(folders)):
        _, refusal = parse(folder, files[folder])
        found.append(refusal or f"{folder}.toml names no folder of tests/qa/scenarios")
    return found
