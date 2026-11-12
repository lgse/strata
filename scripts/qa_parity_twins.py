#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Checks the labelled twins before scripts/qa-parity.sh selects from the parity ledger:
everything tests/qa/twins.rs refuses for the runner's tests, this refuses too.

A run that cannot pass on the Rune target under a named exception is `parallel` in the
ledger, and a twin stands in for it there: `<scenario>-rune.jsonl`, beside the scenario,
with `"targets":["rune"]`. Both headers carry a `parity` label that names the other file,
the exception and its reference (docs/testing.md, "Twins").

Both readers are held to the second corpus of tests/qa/parity_cases.toml, and a problem is
the same line from both.
"""

from __future__ import annotations

import json
from pathlib import Path

import qa_parity_check
from qa_parity_check import PACKET, SUFFIX, exception_problem, packet_entry, reference_problem

# The Rune mutation patches, each named for the scenario that must catch it.
PATCHES = "tests/qa/parity/mutations"
TARGETS = ("gtk", "rune")
# A label's keys: the first three are on every label, and `differs` on the twin's.
LABEL = ("twin", "exception", "ref", "differs")
# Every break `str.splitlines` splits on: LINE_BREAKS in tests/qa/scenario.rs.
LINE_BREAKS = "\n\r\x0b\x0c\x1c\x1d\x1e\x85  "


def one_line(text: str | None) -> bool:
    return bool(text and text.strip()) and not any(mark in text for mark in LINE_BREAKS)


def mislabelled(label: object) -> str | None:
    """What keeps a `parity` label from being read at all."""

    if not isinstance(label, dict):
        return '"parity" is no object'
    if stray := [key for key in label if key not in LABEL]:
        return f'"parity" has the unknown key {stray[0]}'
    if lacking := [key for key in LABEL[:3] if not isinstance(label.get(key), str)]:
        return f'"parity" has no {lacking[0]} that is text'
    differs = label.get("differs")
    return None if differs is None or isinstance(differs, str) else '"parity" has a differs that is no text'


def misshapen(header: dict) -> str | None:
    """What keeps a header's `parity` and `targets` from being read at all."""

    if not isinstance(header.get("scenario"), str) or not isinstance(header.get("modes"), list):
        return "it names no scenario and modes"
    label, targets = header.get("parity"), header.get("targets")
    listed = isinstance(targets, list) and all(target in TARGETS for target in targets)
    if targets is not None and not listed:
        return f'"targets" lists gtk and rune, not {json.dumps(targets)}'
    return None if label is None else mislabelled(label)


def read(text: str) -> tuple[dict | None, str | None]:
    """A header, or why it cannot be read."""

    try:
        header = json.loads(text)
    except json.JSONDecodeError as error:
        return None, f"line 1, the header: {error}"
    refusal = misshapen(header) if isinstance(header, dict) else "it is no object"
    return (None, f"line 1, the header: {refusal}") if refusal else (header, None)


def label_problem(header: dict) -> str | None:
    """What is wrong with a header's `parity` and `targets`, read alone. A name that ends in
    `-rune` is a twin's and no other scenario's."""

    name, label = header["scenario"], header.get("parity")
    original = name.removesuffix(SUFFIX) if name.endswith(SUFFIX) else None
    if label is None:
        return unlabelled(header, original)
    found = exception_problem(label["exception"]) or reference_problem(label["exception"], label["ref"])
    if found or original is None:
        return found or original_problem(header, label)
    return twin_problem(header, label, original)


def unlabelled(header: dict, original: str | None) -> str | None:
    if original is not None:
        return (f'is named as a twin and has no "parity" label: a twin names its original, '
                f"{json.dumps(original)}, with the exception, the reference and what differs")
    if header.get("targets") is not None:
        return ('has "targets" and is no twin: every scenario runs on both targets, and a run that '
                "cannot pass on Rune is pending, parallel with a twin, or excluded by a ruling")
    return None


def twin_problem(header: dict, label: dict, original: str) -> str | None:
    name = header["scenario"]
    if original.endswith(SUFFIX):
        return (f"is a twin of a twin: {json.dumps(original)} is itself a twin, and a twin stands in "
                "for runs of the file both targets share")
    if label["twin"] != original:
        return (f"names {json.dumps(label['twin'])}, and is the twin of {json.dumps(original)}: a twin "
                f"is its original's name with {SUFFIX}, beside it")
    if not one_line(label.get("differs")):
        return f'is a twin without a one-line "differs": say which steps differ from {original}\'s and why'
    if header.get("targets") != ["rune"]:
        return 'is a twin that also runs on GTK: its header needs "targets":["rune"]'
    if header.get("mutation") is not None:
        return (f'is a twin with a "mutation", which names a GTK patch: a twin\'s proof that it can '
                f"fail is {PATCHES}/{name}.patch")
    if header.get("known_bug") is not None:
        return "is a twin marked known_bug: a twin passes on Rune, and a run whose twin cannot is pending"
    return None


def original_problem(header: dict, label: dict) -> str | None:
    name = header["scenario"]
    if label["twin"] != f"{name}{SUFFIX}":
        return (f"names {json.dumps(label['twin'])} as its twin: the twin of {name} is {name}{SUFFIX}, "
                "beside it")
    if label.get("differs") is not None:
        return (f'has "differs", which only the twin carries: {name}{SUFFIX}\'s header says which '
                "steps differ")
    if header.get("targets") is not None:
        return ('has "targets", and only a twin does: the original keeps running on both targets, '
                'and the "parity" label is the one edit made to it')
    return None


def start(header: dict, variant: dict) -> tuple[tuple[str, object], ...]:
    """What a run starts from, by header key: the fixture, the preferences and the layout a
    twin keeps from its original. `start` in tests/qa/twins.rs."""

    def of(source: dict, key: str, default: object) -> object:
        value = source.get(key)
        return default if value is None else value

    return (
        ("tree", of(variant, "tree", of(header, "tree", None))),
        ("images", of(header, "images", {})),
        ("fixture_root", of(header, "fixture_root", None)),
        ("setup", of(header, "setup", []) + of(variant, "setup", [])),
        ("teardown", of(header, "teardown", []) + of(variant, "teardown", [])),
        ("prefs", {**of(header, "prefs", {}), **of(variant, "prefs", {})}),
        ("layout", of(variant, "layout", of(header, "layout", "us"))),
        ("portal", of(header, "portal", False)),
    )


def same(first: object, second: object) -> bool:
    """Whether two header values are one JSON value: 1 is not 1.0, and not true."""

    return json.dumps(first, sort_keys=True) == json.dumps(second, sort_keys=True)


def facts(folder: str, scenario: str, text: str) -> dict:
    """What a header says of its scenario, for the pair checks."""

    header, refusal = read(text)
    planned = qa_parity_check.variant_runs(header) if header else []
    return {"folder": folder, "scenario": scenario, "header": header, "refusal": refusal,
            "fault": label_problem(header) if header else None,
            "runs": [(mode, label, start(header, variant)) for mode, label, variant in planned]}


def file_of(scenario: dict) -> str:
    """The file under tests/qa, as a problem names it."""

    return f"scenarios/{scenario['folder']}/{scenario['scenario']}.jsonl"


def assemble(scenarios: list[dict], patches: set[str], packet: set[str]) -> dict:
    """What the ledger and the labels are checked against: every scenario header in the
    runner's order, the scenarios the Rune mutation patches are named for, and the file
    names of the Rune filing packet."""

    return {"scenarios": sorted(scenarios, key=file_of), "patches": patches, "packet": packet}


def corpus(scenarios: Path, patches: Path, packet: Path) -> dict:
    """The corpus of a checkout: its scenario folders, its Rune patches and its packet."""

    files = qa_parity_check.scenario_files(scenarios)
    return assemble(
        [facts(path.parent.name, path.stem, qa_parity_check.header_line(path)) for path in files],
        {path.stem for path in patches.glob("*.patch")},
        {path.name for path in packet.iterdir()} if packet.is_dir() else set())


def planned(corpus: dict) -> list[dict]:
    """Every run the ledger lists, as the runner plans them: no twin's."""

    return [{"folder": scenario["folder"], "scenario": scenario["scenario"], "run": label,
             "gate": qa_parity_check.gate(scenario["folder"], scenario["scenario"], mode, label)}
            for scenario in corpus["scenarios"] if not scenario["scenario"].endswith(SUFFIX)
            for mode, label, _ in scenario["runs"]]


def sound(scenario: dict) -> bool:
    """Whether nothing is wrong with the header alone: a pair is checked only then."""

    return scenario["header"] is not None and scenario["fault"] is None


def alone(scenario: dict) -> str | None:
    """What is wrong with the label, read alone."""

    return f"{file_of(scenario)} {scenario['fault']}" if scenario["fault"] else None


def unread(corpus: dict) -> list[str]:
    """The headers that cannot be read, one line each. Their runs are unknown, so nothing is
    checked against a corpus that has one."""

    return [f"{file_of(scenario)}: {scenario['refusal']}"
            for scenario in corpus["scenarios"] if scenario["refusal"]]


def find(corpus: dict, name: str) -> dict | None:
    return next((scenario for scenario in corpus["scenarios"] if scenario["scenario"] == name), None)


def orphaned(corpus: dict, twin: dict, original: str) -> list[str]:
    """What is wrong with a twin whose original is not beside it."""

    found = find(corpus, original)
    if found is None:
        return [f"{file_of(twin)} is the twin of {original}, and no scenario has that name: a twin "
                "stands beside its original"]
    if found["folder"] != twin["folder"]:
        return [f"{file_of(twin)} is the twin of {original}, which is in {found['folder']}: a twin is "
                "a file beside its original"]
    return []


def unpaired(scenario: dict, entries: list[dict]) -> list[str]:
    """What is wrong with a scenario that has no twin beside it: a parallel run, or a label."""

    folder, name = scenario["folder"], scenario["scenario"]
    missing = f"tests/qa/scenarios/{folder}/{name}{SUFFIX}.jsonl"
    parallel = [f"{folder}.toml: {name}/{entry['run']} is parallel, and its twin's file does not "
                f"exist: {missing}. Write the twin, or the run is pending"
                for entry in entries if entry["status"] == "parallel"]
    if parallel or scenario["header"].get("parity") is None:
        return parallel
    return [f"{file_of(scenario)} names the twin {name}{SUFFIX}, and {missing} does not exist: write "
            'the twin, or take the "parity" label off']


def disagreements(label: dict, twin: dict) -> list[str]:
    """Where the twin's label parts from its original's."""

    found = []
    if label["exception"] != twin["exception"]:
        found.append(f"is under the exception {json.dumps(twin['exception'])} and its original under "
                     f"{json.dumps(label['exception'])}: the two headers carry one code")
    if label["ref"] != twin["ref"]:
        found.append(f"cites {json.dumps(twin['ref'])} and its original {json.dumps(label['ref'])}: "
                     "the two headers carry one reference")
    return found


def unfounded(corpus: dict, twin: dict, label: dict) -> list[str]:
    """What a twin lacks outside its header: the packet entry a `rune` twin expires with, and
    the patch that proves the twin can fail."""

    found, name = [], twin["scenario"]
    entry = packet_entry(label["ref"]) if label["exception"] == "rune" else None
    if entry is not None and entry not in corpus["packet"]:
        found.append(f"cites {json.dumps(label['ref'])}, and {PACKET} has no such entry: a rune twin "
                     "names the packet entry it expires with")
    if name not in corpus["patches"]:
        found.append(f"has no proof that it can fail: add {PATCHES}/{name}.patch, which breaks the "
                     "behaviour in crates/strata-app, and check it with ./scripts/qa-parity.sh mutations")
    return found


def stray(original: dict, label: str, begins: tuple) -> str | None:
    """What is wrong with one of a twin's runs: its original lacks it, or starts it from
    something else."""

    name = original["scenario"]
    shared = next((start for _, other, start in original["runs"] if other == label), None)
    if shared is None:
        return (f"has the run {label}, which {name} lacks: a twin holds only modes and variants of "
                "its original")
    key = next((key for (key, value), (_, own) in zip(shared, begins) if not same(value, own)), None)
    return None if key is None else (
        f"starts {label} with another {json.dumps(key)} than {name}: a twin keeps what its original "
        'starts from, and only the steps "differs" names differ')


def standing(twin: dict, entry: dict, label: dict) -> str | None:
    """What is wrong with how an entry of the original stands against the twin's runs."""

    name, run = twin["scenario"], entry["run"]
    stands_in = any(other == run for _, other, _ in twin["runs"])
    parallel = entry["status"] == "parallel"
    if stands_in and not parallel:
        return (f"is {entry['status']}, and {name}/{run} stands in for it: a run with a twin is "
                "parallel, with the exception, the twin and the reference")
    if parallel and not stands_in:
        return f"is parallel, and {name} has no run {run}: the twin holds every run it stands in for"
    return disagreement(entry, label) if parallel else None


def disagreement(entry: dict, label: dict) -> str | None:
    """Where a parallel entry parts from its scenario's label. An entry without the field, or
    with one of the wrong form, is the ledger's own problem and is said once, there."""

    code, reference = entry.get("exception"), entry.get("ref")
    if code is None or exception_problem(code):
        return None
    if reference is not None and reference_problem(code, reference):
        reference = None
    if code != label["exception"]:
        return (f"is parallel under {json.dumps(code)}, and its header says "
                f"{json.dumps(label['exception'])}: the ledger and the two headers carry one code")
    if reference is not None and reference != label["ref"]:
        return (f"cites {json.dumps(reference)}, and its header {json.dumps(label['ref'])}: the "
                "ledger and the two headers carry one reference")
    return None


def paired(corpus: dict, original: dict, twin: dict, entries: list[dict]) -> list[str]:
    """What is wrong with a scenario and the twin beside it, each sound alone."""

    file, name = file_of(twin), twin["scenario"]
    label, twin_label = original["header"].get("parity"), twin["header"]["parity"]
    if label is None:
        return [f'{file_of(original)} has the twin {name} and no "parity" label: give it the label, '
                "with the twin's exception and reference; its steps stay as they are"]
    if not twin["runs"]:
        return [f"{file} has no run: a twin holds the modes or variants of {original['scenario']} it "
                "stands in for"]
    found = disagreements(label, twin_label) + unfounded(corpus, twin, twin_label)
    found += [problem for _, run, begins in twin["runs"] if (problem := stray(original, run, begins))]
    found = [f"{file} {problem}" for problem in found]
    return found + [f"{original['folder']}.toml: {qa_parity_check.name(entry)} {problem}"
                    for entry in entries if (problem := standing(twin, entry, label))]


def pair_problems(corpus: dict, scenario: dict, entries: dict[str, list[dict]]) -> list[str]:
    if not sound(scenario):
        return []
    name = scenario["scenario"]
    if name.endswith(SUFFIX):
        return orphaned(corpus, scenario, name.removesuffix(SUFFIX))
    own = [entry for entry in entries.get(scenario["folder"], []) if entry["scenario"] == name]
    twin = find(corpus, f"{name}{SUFFIX}")
    if twin is None:
        return unpaired(scenario, own)
    # A twin in another folder is reported from its own side.
    beside = twin["folder"] == scenario["folder"] and sound(twin)
    return paired(corpus, scenario, twin, own) if beside else []


def problems(corpus: dict, entries: dict[str, list[dict]]) -> list[str]:
    """Everything wrong with the labels and the pairs, one line each: first each header alone,
    then each pair against its two headers, the ledger, the patches and the packet.
    `entries` is each folder's entries, for the files that can be read."""

    found = [problem for scenario in corpus["scenarios"] if (problem := alone(scenario))]
    for scenario in corpus["scenarios"]:
        found += pair_problems(corpus, scenario, entries)
    return found


def faults(files: dict[str, str], corpus: dict) -> list[str]:
    """Everything wrong with the ledger and the labelled pairs against a corpus: `faults` in
    tests/qa/parity.rs. `files` is each folder's file text."""

    if refused := unread(corpus):
        return refused
    parsed = {folder: qa_parity_check.parse(folder, text) for folder, text in files.items()}
    entries = {folder: entries for folder, (entries, refusal) in parsed.items() if not refusal}
    return qa_parity_check.problems(files, planned(corpus)) + problems(corpus, entries)
