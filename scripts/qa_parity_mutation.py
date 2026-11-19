#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Proves the Rune target's parity runs can fail: each patch under tests/qa/parity/mutations
breaks one behaviour of strata-app, and the scenario it is named for must catch it.

Checked the way scripts/qa_mutation.py checks the GTK application's patches: the scenario's
runs pass on the Rune target unpatched, and at least one fails a step with the patch
applied. A patch is `<scenario>.patch`, or `<scenario>.<label>.patch` when the scenario
catches more than one behaviour's breaking (the owner's ruling of 2026-11-19): each is
proven on its own, by the scenario's runs. The label is lowercase words joined by hyphens.

A patch may be named for a scenario of any folder: its runs are looked up in the whole
ledger, tests/qa/parity/<folder>.toml. They are its required runs. A twin's are the runs
that stand in for its original's parallel ones, and `<scenario>-rune.patch` is the proof
that the twin can fail, which every twin has.

Named patches run alone: `qa_parity_mutation.py rune <patch>...`, each named as its scenario
(every patch of it), as the patch's name, `<scenario>.patch` or `<scenario>.<label>.patch`, or
the patch's path, proves those once, as a patch written, changed or
refreshed is proven (docs/rune/parity/README.md, "When a mutation run is right").

With `gtk` it runs the GTK application's own patches for the slice's exit check instead:
every patch under tests/qa/mutations but the ones the parity ledger names as stale before
the slice, in tests/qa/parity/slice.toml, which must be stale still.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import time
from pathlib import Path

import qa_mutation
import qa_parity
from qa_mutation import (
    Mutation, Report, Scenario, baseline_verdict, mutated_verdict, own_runs, parse_qa_log,
    write_summary,
)

REPOSITORY = Path(__file__).resolve().parents[1]
MUTATIONS = REPOSITORY / "tests/qa/parity/mutations"
OUTPUT = REPOSITORY / "target/qa-parity-mutation"


LABEL = re.compile(r"[a-z0-9]+(-[a-z0-9]+)*")


def patches(directory: Path | None = None) -> list[str]:
    """The patches' names: `<scenario>` or `<scenario>.<label>`."""

    return sorted(path.stem for path in (directory or MUTATIONS).glob("*.patch"))


def scenario_of(patch: str) -> str:
    """The scenario a patch is named for: its name up to a label's dot."""

    return patch.split(".", 1)[0]


def malformed(available: list[str]) -> list[str]:
    """What is wrong with the patches' names: a label is lowercase words joined by hyphens,
    after one dot."""

    problems = []
    for patch in available:
        scenario, dot, label = patch.partition(".")
        if not scenario or (dot and not LABEL.fullmatch(label)):
            problems.append(f"{patch}.patch is named neither <scenario>.patch nor "
                            "<scenario>.<label>.patch, a label being lowercase words joined by "
                            "hyphens")
    return problems


def in_mutations(name: str) -> bool:
    """Whether `name` is a bare name, or a path to a file of the Rune patches' directory, from
    here or from the repository: GTK's `tests/qa/mutations/<scenario>.patch` is not one."""

    if "/" not in name:
        return True
    path = Path(name)
    bases = [Path()] if path.is_absolute() else [Path.cwd(), REPOSITORY]
    return any((base / path).resolve().parent == MUTATIONS.resolve() for base in bases)


def chosen(names: list[str], available: list[str]) -> tuple[list[str], list[str]]:
    """The patches `names` select, in the order given and each once, and what is wrong with
    the names that select none. A scenario's bare name selects every patch of it; a patch's
    file name or path, or a labelled patch's bare name, that patch alone. No names select
    every patch."""

    if not names:
        return list(available), []
    picked, problems = [], []
    for name in names:
        stem = Path(name).name.removesuffix(".patch")
        alone = name.endswith(".patch") or "/" in name or "." in stem
        selected = ([stem] if stem in available else []) if alone else [
            patch for patch in available if scenario_of(patch) == stem]
        if not in_mutations(name):
            problems.append(f"{name!r} is no Rune mutation patch: a path names a patch under "
                            f"{MUTATIONS.relative_to(REPOSITORY)}")
        elif not selected:
            problems.append(f"no Rune mutation patch {name!r}: the patches under "
                            f"{MUTATIONS.relative_to(REPOSITORY)} are {', '.join(available)}")
        else:
            picked += [patch for patch in selected if patch not in picked]
    return picked, problems


def guards(scenario: str, ledger: dict) -> list[str]:
    """The scenario's own runs that the ledger runs on the Rune target: its required runs,
    or, for a twin, the runs that stand in for parallel ones."""

    on_rune = qa_parity.deciding(ledger, qa_parity.EVERY, "rune", scenario)
    return [run for run in on_rune if run.split("/", 1)[0] == scenario]


def unguarded(patches: list[str], ledger: dict) -> list[str]:
    """The patches whose scenario has no run on the Rune target: nothing would be run to
    catch them. A scenario whose runs are all parallel has none: the patch is its twin's."""

    return [patch for patch in patches if not guards(scenario_of(patch), ledger)]


def run_rune(scenario: str, destination: Path) -> tuple[dict[str, str], int]:
    """The scenario's runs at parity on the Rune target, in the pinned image."""

    destination.mkdir(parents=True, exist_ok=True)
    log = destination / "qa-parity.log"
    started = time.monotonic()
    command = [str(REPOSITORY / "scripts/qa-parity.sh"), qa_parity.EVERY, "--target", "rune",
               "--scenario", scenario]
    with log.open("w", encoding="utf-8") as output:
        status = subprocess.run(
            command, cwd=REPOSITORY, stdout=output, stderr=subprocess.STDOUT, check=False,
        ).returncode
    print(f"   qa-parity.sh exited {status} after {time.monotonic() - started:.0f}s; "
          f"log {log.relative_to(REPOSITORY)}", flush=True)
    return parse_qa_log(log.read_text(encoding="utf-8", errors="replace")), status


def check(names: list[str] | None = None) -> int:
    started = time.monotonic()
    available = patches()
    if not available:
        print(f"No patch under {MUTATIONS.relative_to(REPOSITORY)}.", file=sys.stderr)
        return 2
    problems = malformed(available)
    scenarios, chosen_problems = chosen(names or [], available)
    problems += chosen_problems
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 2
    if missing := unguarded(scenarios, qa_parity.load()):
        print(f"No run on the Rune target in the ledger for: {', '.join(missing)}. A patch is "
              "named for a scenario with a required run, or for a twin.", file=sys.stderr)
        return 2
    (OUTPUT / "summary.md").unlink(missing_ok=True)
    reports = []
    for patch in scenarios:
        scenario = scenario_of(patch)
        print(f"== {patch}: unmodified tree", flush=True)
        runs, _ = run_rune(scenario, OUTPUT / "baseline" / patch)
        report = Report(Scenario(scenario, f"{patch}.patch"),
                        baseline_verdict(own_runs(scenario, runs)))
        reports.append(report)
        if not report.baseline.caught:
            print(f"== {patch}: skipped, {report.baseline.note}", flush=True)
            continue
        print(f"== {patch}: mutated tree", flush=True)
        with Mutation(patch, mutations=MUTATIONS):
            runs, status = run_rune(scenario, OUTPUT / "mutations" / patch)
        report.runs = own_runs(scenario, runs)
        report.mutated = mutated_verdict(report.runs, status)
    summary = write_summary(reports, time.monotonic() - started, OUTPUT)
    print("\n" + summary)
    return 0 if all(report.caught for report in reports) else 1


def applies(patch: str) -> bool:
    """Whether a GTK mutation patch still applies to this checkout."""

    path = qa_mutation.MUTATIONS / f"{patch}.patch"
    return qa_mutation.git("apply", "--check", str(path)).returncode == 0


def sort_gtk_patches(patches: list[str], stale: list[str], applies=applies) -> tuple[list[str], list[str]]:
    """The GTK patches to run, and what is wrong with the ones left out.

    A patch is left out only when it does not apply and the ledger names it. One that fails
    to apply unnamed, or is named and applies again, is a problem: the ledger is not a place
    to park a patch that works.
    """

    broken = [patch for patch in patches if not applies(patch)]
    problems = [f"{patch}.patch does not apply, and the ledger does not name it stale"
                for patch in broken if patch not in stale]
    problems += [f"{patch}.patch applies again: remove its [[stale_patch]] entry from the ledger"
                 for patch in stale if patch in patches and patch not in broken]
    problems += [f"{patch}.patch is named stale, and no scenario names it as its mutation"
                 for patch in stale if patch not in patches]
    return [patch for patch in patches if patch not in broken], problems


def check_gtk(notes: Path | None = None) -> int:
    """The exit check's fourth item: the GTK patches behave as they did before the slice."""

    patches = [scenario.mutation for scenario in qa_mutation.chosen_scenarios([])]
    stale = qa_parity.stale_patches(qa_parity.load())
    chosen, problems = sort_gtk_patches(patches, stale)
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 2
    for patch in stale:
        line = (f"ALLOWANCE USED: {patch}.patch does not apply, as before the slice; "
                f"the other {len(chosen)} GTK mutation patches run.")
        print(line, flush=True)
        if notes:
            with notes.open("a", encoding="utf-8") as stream:
                stream.write(line + "\n")
    return qa_mutation.check(chosen)


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("target", nargs="?", choices=("rune", "gtk"), default="rune",
                        help="rune: the patches under tests/qa/parity/mutations (the default); "
                             "gtk: the GTK application's, less the ledger's stale ones")
    parser.add_argument("patches", nargs="*", metavar="patch",
                        help="rune only: the patches to run alone, each named as its scenario "
                             "(every patch of it), <scenario>.patch, <scenario>.<label>.patch or "
                             "its path; none runs every patch")
    parser.add_argument("--notes", type=Path,
                        help="a file to append each allowance used to, for the caller's summary")
    options = parser.parse_args(arguments)
    if os.environ.get("STRATA_QA_UPDATE_BASELINES") == "1":
        print("STRATA_QA_UPDATE_BASELINES must be unset for a mutated run", file=sys.stderr)
        return 2
    if options.target == "gtk" and options.patches:
        print("gtk runs every GTK patch; one alone is ./scripts/qa-mutation.sh <scenario>",
              file=sys.stderr)
        return 2
    try:
        return check_gtk(options.notes) if options.target == "gtk" else check(options.patches)
    except qa_parity.BadLedger as error:
        return qa_parity.refuse(qa_parity.LEDGER, error)
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(main())
