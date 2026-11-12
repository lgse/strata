#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Checks the labelled twins before scripts/qa-parity.sh selects from the parity ledger:
everything tests/qa/twins.rs refuses for the runner's tests, this refuses too.

A run that cannot pass on the Rune target under a named exception is `parallel` in the
ledger, and a twin stands in for it there: `<scenario>-rune.jsonl`, beside the scenario,
with `"targets":["rune"]`. Both headers carry a `parity` label that names the other file,
the exception and its reference (docs/testing.md, "Twins").

Both readers are held to one table, tests/qa/parity_pair_cases.toml, and a problem is
the same line from both.
"""

from __future__ import annotations

import json
from collections import Counter
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


def is_text(value: object) -> bool:
    return isinstance(value, str)


def is_flag(value: object) -> bool:
    return isinstance(value, bool)


def is_list(value: object) -> bool:
    return isinstance(value, list)


def is_table(value: object) -> bool:
    return isinstance(value, dict)


def is_count(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def texts(value: object) -> bool:
    return is_list(value) and all(is_text(item) for item in value)


def named_texts(value: object) -> bool:
    return is_table(value) and all(is_text(item) for item in value.values())


def numbers(value: object) -> bool:
    return is_table(value) and all(isinstance(item, (int, float)) and not is_flag(item)
                                   for item in value.values())


def anything(_value: object) -> bool:
    return True


# The keys of a scenario header and of a variant, as `Header` and `Variant` in
# tests/qa/scenario.rs have them: what a value is, whether the key may be left out, and
# whether it may be null. The runner's reader refuses a key it does not have and a value of
# another shape, and so does this one.
SHAPES = {
    "scenario": (is_text, "text", False, False),
    "modes": (texts, "a list of modes", False, False),
    "prefs": (is_table, "an object", True, False),
    "layout": (is_text, "text", True, False),
    "mutation": (is_text, "text", True, True),
    "known_bug": (is_text, "text", True, True),
    "known_bug_step": (is_count, "a line number", True, True),
    "variants": (is_list, "a list", True, False),
    "tree": (anything, "a tree", True, True),
    "images": (is_table, "an object", True, False),
    "gtk_settings": (named_texts, "an object of text", True, False),
    "setup": (is_list, "a list", True, False),
    "teardown": (is_list, "a list", True, False),
    "env": (named_texts, "an object of text", True, False),
    "session_bus": (is_flag, "true or false", True, False),
    "portal": (is_flag, "true or false", True, False),
    "input_method": (is_text, "text", True, True),
    "fixture_root": (is_text, "text", True, True),
    "skip": (is_text, "text", True, True),
    "cost": (numbers, "an object of numbers", True, True),
    "quality": (anything, "a rating", True, True),
    "parity": (is_table, "an object", True, True),
    "targets": (is_list, "a list", True, True),
}
VARIANT_SHAPES = {
    "id": (is_text, "text", False, False),
    "modes": (texts, "a list of modes", True, True),
    "prefs": (is_table, "an object", True, False),
    "layout": (is_text, "text", True, True),
    "vars": (named_texts, "an object of text", True, False),
    "tree": (anything, "a tree", True, True),
    "setup": (is_list, "a list", True, False),
    "teardown": (is_list, "a list", True, False),
}


def misfit(table: dict, key: str, shape: tuple, what: str) -> str | None:
    """What the runner would not read of one key: that it is missing, or of another shape."""

    fits, described, optional, nullable = shape
    if key not in table:
        return None if optional else f"{what} has no {key}"
    allowed = (nullable and table[key] is None) or fits(table[key])
    return None if allowed else f'"{key}" of {what} is not {described}'


def unshaped(table: dict, shapes: dict, what: str) -> str | None:
    """The first key of a header or a variant that the runner would not read: one it does
    not have, one it needs, or a value of another shape."""

    if stray_keys := [key for key in table if key not in shapes]:
        return f"{what} has the unknown key {stray_keys[0]}"
    found = (misfit(table, key, shape, what) for key, shape in shapes.items())
    return next((problem for problem in found if problem), None)


def mislabelled(label: dict) -> str | None:
    """What keeps a `parity` label from being read at all."""

    if stray := [key for key in label if key not in LABEL]:
        return f'"parity" has the unknown key {stray[0]}'
    if lacking := [key for key in LABEL[:3] if not is_text(label.get(key))]:
        return f'"parity" has no {lacking[0]} that is text'
    differs = label.get("differs")
    return None if differs is None or is_text(differs) else '"parity" has a differs that is no text'


def misshapen(header: dict) -> str | None:
    """What keeps a header from being read at all, as the runner reads it."""

    if found := unshaped(header, SHAPES, "the header"):
        return found
    for variant in header.get("variants", []):
        found = unshaped(variant, VARIANT_SHAPES, "a variant") if is_table(variant) else "a variant is no object"
        if found:
            return found
    targets = header.get("targets")
    if targets is not None and any(target not in TARGETS for target in targets):
        return f'"targets" lists gtk and rune, not {json.dumps(targets)}'
    label = header.get("parity")
    return None if label is None else mislabelled(label)


def lines(text: str) -> list[tuple[int, str]]:
    """A scenario file's lines with anything on them, by number: the header is the first."""

    return [(number, line) for number, line in enumerate(text.split("\n"), start=1) if line.strip()]


def read(text: str) -> tuple[dict | None, list, str | None]:
    """A scenario file's header and, for a labelled scenario, its steps by line, which only
    a pair is compared by; or why the file cannot be read."""

    first, *rest = lines(text) or [(1, "")]
    try:
        header = json.loads(first[1])
    except json.JSONDecodeError as error:
        return None, [], f"line 1, the header: {error}"
    refusal = misshapen(header) if is_table(header) else "it is no object"
    if refusal:
        return None, [], f"line 1, the header: {refusal}"
    steps = []
    for number, line in rest if header.get("parity") is not None else []:
        try:
            step = json.loads(line)
        except json.JSONDecodeError as error:
            return None, [], f"line {number}: {error}"
        if not is_table(step):
            return None, [], f"line {number}: a step must be an object"
        steps.append((number, step))
    return header, steps, None


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
        ("env", of(header, "env", {})),
        ("session_bus", of(header, "session_bus", False)),
        ("input_method", of(header, "input_method", None)),
    )


# What a step says on failure and how long it may take are no part of what it does or
# asserts, so two steps that differ only there are one step: UNCOMPARED in twins.rs.
UNCOMPARED = ("message", "timeout_ms")


def token_length(text: str) -> int:
    """How much of the text after a `$` is a variable's name."""

    named = (index for index, character in enumerate(text)
             if not (character.isascii() and (character.isalnum() or character == "_")))
    return next(named, len(text))


def substitute_text(text: str, variables: dict[str, str]) -> str:
    """`substitute_text` in tests/qa/variables.rs: `$name` is a whole token, and `$$` stays."""

    output, rest = [], text
    while (at := rest.find("$")) >= 0:
        output.append(rest[:at])
        after = rest[at + 1:]
        if after.startswith("$"):
            output.append("$$")
            rest = after[1:]
            continue
        token = after[:token_length(after)]
        output.append(variables.get(token, f"${token}") if token else "$")
        rest = after[len(token):]
    return "".join(output + [rest])


def substitute(value: object, variables: dict[str, str]) -> object:
    """A step with the variant's `vars` read in: `substitute` in tests/qa/variables.rs."""

    if isinstance(value, list):
        return [substitute(item, variables) for item in value]
    if isinstance(value, dict):
        return {key: substitute(item, variables) for key, item in value.items()}
    return substitute_text(value, variables) if isinstance(value, str) else value


def run_steps(steps: list, mode: str, variant: dict) -> list:
    """The steps of one run by line, as the runner picks them: the variant's `vars` read in,
    and the steps of other modes and variants left out. `run_steps` in tests/qa/twins.rs."""

    def selects(step: dict, key: str, name: str) -> bool:
        listed = step.pop(key, None)
        return listed is None or (isinstance(listed, list) and name in listed)

    picked = []
    for number, step in steps:
        step = substitute(step, variant.get("vars", {}))
        if selects(step, "modes", mode) and selects(step, "variants", variant.get("id", "")):
            picked.append((number, {key: value for key, value in step.items() if key not in UNCOMPARED}))
    return picked


def reads_pref(value: object) -> bool:
    """Whether a predicate reads settings.toml: a `pref` anywhere in it."""

    if isinstance(value, dict):
        return any(key == "pref" or reads_pref(item) for key, item in value.items())
    return isinstance(value, list) and any(reads_pref(item) for item in value)


def end_state(step: dict) -> str | None:
    """What a step is among those a twin must keep: the runner's `fs` steps, which change and
    check the files on disk, and the checks that read settings.toml."""

    if "fs" in step:
        return "an fs step"
    return "a pref check" if any(reads_pref(step[key]) for key in ("wait", "expect") if key in step) else None


def dropped(shared: list, own: list) -> list[tuple[int, str]]:
    """The end-state steps of `shared` that `own` lacks, or has out of the original's order:
    each by its line and what it is."""

    start_at, missing = 0, []
    for number, step in shared:
        kind = end_state(step)
        if kind is None:
            continue
        at = next((index for index in range(start_at, len(own)) if same(own[index][1], step)), None)
        if at is None:
            missing.append((number, kind))
        else:
            start_at = at + 1
    return missing


def tally(shared: list, own: list) -> tuple[int, int, int]:
    """How a twin's run stands to its original's: the steps that are the original's, the
    twin's own, and the original's that it drops."""

    theirs = Counter(json.dumps(step, sort_keys=True) for _, step in shared)
    ours = Counter(json.dumps(step, sort_keys=True) for _, step in own)
    kept = sum((theirs & ours).values())
    return kept, len(own) - kept, len(shared) - kept


def same(first: object, second: object) -> bool:
    """Whether two header values are one JSON value: 1 is not 1.0, and not true."""

    return json.dumps(first, sort_keys=True) == json.dumps(second, sort_keys=True)


def facts(folder: str, scenario: str, text: str) -> dict:
    """What a header says of its scenario, for the pair checks."""

    header, steps, refusal = read(text)
    planned = qa_parity_check.variant_runs(header) if header else []
    return {"folder": folder, "scenario": scenario, "header": header, "refusal": refusal,
            "fault": label_problem(header) if header else None,
            "runs": [(mode, label, start(header, variant)) for mode, label, variant in planned],
            "steps": {label: run_steps(steps, mode, variant) for mode, label, variant in planned}}


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
        [facts(path.parent.name, path.stem, path.read_text(encoding="utf-8")) for path in files],
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
    """The files that cannot be read, one line each: a header, or a step of a labelled
    scenario. Their runs are unknown, so nothing is checked against a corpus that has one."""

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


def strayed_steps(original: dict, twin: dict, label: str) -> list[str]:
    """What is wrong with the steps of one of a twin's runs: none, its original's and no
    other, or without an end-state step its original has."""

    name = original["scenario"]
    theirs, ours = original["steps"][label], twin["steps"][label]
    if not ours:
        return [f"has no step in {label}: a twin reaches its original's result by steps of its own"]
    if same([step for _, step in theirs], [step for _, step in ours]):
        return [f"runs {label} by its original's steps, so nothing differs: a run the shared file "
                "passes on Rune is required and has no twin"]
    return [f"lacks line {number} of {name} in {label}, {kind}: a twin keeps every fs step and every "
            "pref check of its original, in its order, for the files on disk and settings.toml match "
            "with no exception" for number, kind in dropped(theirs, ours)]


def stray(original: dict, twin: dict, label: str, begins: tuple) -> list[str]:
    """What is wrong with one of a twin's runs: its original lacks it, starts it from
    something else, or its steps do not stand to its original's as a twin's do."""

    name = original["scenario"]
    shared = next((start for _, other, start in original["runs"] if other == label), None)
    if shared is None:
        return [f"has the run {label}, which {name} lacks: a twin holds only modes and variants of "
                "its original"]
    key = next((key for (key, value), (_, own) in zip(shared, begins) if not same(value, own)), None)
    if key is not None:
        return [f"starts {label} with another {json.dumps(key)} than {name}: a twin keeps what its "
                'original starts from, and only the steps "differs" names differ']
    return strayed_steps(original, twin, label)


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
    found += [problem for _, run, begins in twin["runs"] for problem in stray(original, twin, run, begins)]
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


def twins(corpus: dict) -> dict[str, dict]:
    """Each twin of a corpus that is in order, by its name: what its header says differs, and
    how its steps stand to its original's over the runs it holds."""

    listed = {}
    for twin in corpus["scenarios"]:
        original = find(corpus, twin["scenario"].removesuffix(SUFFIX))
        if not twin["scenario"].endswith(SUFFIX) or original is None:
            continue
        tallies = [tally(original["steps"][label], twin["steps"][label]) for _, label, _ in twin["runs"]]
        listed[twin["scenario"]] = {"differs": twin["header"]["parity"]["differs"],
                                    "steps": tuple(sum(column) for column in zip(*tallies))}
    return listed


def faults(files: dict[str, str], corpus: dict) -> list[str]:
    """Everything wrong with the ledger and the labelled pairs against a corpus: `faults` in
    tests/qa/parity.rs. `files` is each folder's file text."""

    if refused := unread(corpus):
        return refused
    parsed = {folder: qa_parity_check.parse(folder, text) for folder, text in files.items()}
    entries = {folder: entries for folder, (entries, refusal) in parsed.items() if not refusal}
    return qa_parity_check.problems(files, planned(corpus)) + problems(corpus, entries)
