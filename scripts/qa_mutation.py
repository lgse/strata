#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Proves each `--qa` mutation patch under tests/qa/mutations is caught by its scenario.

Each patch is named `<scenario>.patch`, or `<scenario>.<label>.patch` when one scenario
catches more than one behaviour's breaking (the label is lowercase words joined by hyphens).
Each patch is checked by running its scenario unmodified (all runs must pass) and then with
the patch applied (at least one run must fail a step and qa.sh must exit non-zero).
"""

from __future__ import annotations

import argparse
import re
import signal
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]
SCENARIOS = REPOSITORY / "tests/qa/scenarios"
MUTATIONS = REPOSITORY / "tests/qa/mutations"
OUTPUT = REPOSITORY / "target/qa-mutation"

LABEL = re.compile(r"[a-z0-9]+(-[a-z0-9]+)*")
RUN = r"(?P<run>[^/\s]+/\S+)"
PLANNED = re.compile(rf"^RUN {RUN}$")
PASSED = re.compile(rf"^PASS {RUN}$")
KNOWN_BUG = re.compile(rf"^KNOWN BUG {RUN} ")
FAILED = re.compile(rf"^{RUN} (?P<outcome>failed:|could not run:|passes but is marked known_bug|is marked known_bug)")


@dataclass(frozen=True)
class PatchTarget:
    scenario: str
    patch: str


def scenario_names(directory: Path | None = None) -> set[str]:
    return {path.stem for path in (directory or SCENARIOS).glob("*/*.jsonl")}


def patches(directory: Path | None = None) -> list[str]:
    """The committed patches' stems: `<scenario>` or `<scenario>.<label>`."""

    return sorted(path.stem for path in (directory or MUTATIONS).glob("*.patch"))


def scenario_of(patch: str) -> str:
    """The scenario a patch is named for: its stem up to a label's dot."""

    return patch.split(".", 1)[0]


def well_named(patch: str) -> bool:
    """Whether a patch's stem is `<scenario>` or `<scenario>.<label>`."""

    scenario, dot, label = patch.partition(".")
    return bool(scenario) and (not dot or LABEL.fullmatch(label) is not None)


def malformed(available: list[str]) -> list[str]:
    return [
        f"{patch}.patch is named neither <scenario>.patch nor <scenario>.<label>.patch, a "
        "label being lowercase words joined by hyphens"
        for patch in available
        if not well_named(patch)
    ]


def in_mutations(name: str, mutations: Path = MUTATIONS) -> bool:
    if "/" not in name:
        return True
    path = Path(name)
    bases = [Path()] if path.is_absolute() else [Path.cwd(), REPOSITORY]
    return any((base / path).resolve().parent == mutations.resolve() for base in bases)


def names_one_patch(name: str, stem: str) -> bool:
    return name.endswith(".patch") or "/" in name or "." in stem


def selection(name: str, available: list[str]) -> list[str]:
    stem = Path(name).name.removesuffix(".patch")
    if names_one_patch(name, stem):
        return [stem] if stem in available else []
    return [patch for patch in available if scenario_of(patch) == stem]


def chosen_patches(
    names: list[str],
    scenarios_dir: Path | None = None,
    mutations_dir: Path | None = None,
) -> list[PatchTarget]:
    """The patches selected by `names`, or every patch under `mutations_dir` when empty."""

    mutations_dir = mutations_dir or MUTATIONS
    available = patches(mutations_dir)
    problems = malformed(available)
    known = scenario_names(scenarios_dir)

    if not names:
        picked = list(available)
    else:
        picked = []
        for name in names:
            selected = selection(name, available)
            if not in_mutations(name, mutations_dir):
                problems.append(
                    f"{name!r} is outside {mutations_dir}"
                )
            elif not selected:
                problems.append(f"{name}: no mutation patch in {mutations_dir}")
            else:
                picked.extend(patch for patch in selected if patch not in picked)

    for patch in picked:
        scenario = scenario_of(patch)
        if scenario not in known:
            problems.append(f"{patch}.patch names unknown QA scenario {scenario!r}")

    if problems:
        raise SystemExit("\n".join(problems))
    return [PatchTarget(scenario=scenario_of(patch), patch=patch) for patch in picked]


def parse_qa_log(text: str) -> dict[str, str]:
    """Run statuses by `scenario/run`; a planned run without a result line stays `missing`."""

    runs: dict[str, str] = {}
    for line in text.splitlines():
        line = line.rstrip()
        if match := PLANNED.match(line):
            runs.setdefault(match["run"], "missing")
        elif match := PASSED.match(line):
            runs[match["run"]] = "pass"
        elif match := KNOWN_BUG.match(line):
            runs[match["run"]] = "known bug"
        elif match := FAILED.match(line):
            runs[match["run"]] = "fail" if match["outcome"] == "failed:" else "error"
    return runs


def own_runs(scenario: str, runs: dict[str, str]) -> dict[str, str]:
    return {run: status for run, status in runs.items() if run.split("/", 1)[0] == scenario}


@dataclass
class Verdict:
    caught: bool
    note: str


def baseline_verdict(runs: dict[str, str]) -> Verdict:
    if not runs:
        return Verdict(False, "the unmodified tree ran nothing")
    bad = [f"{run} {status}" for run, status in runs.items() if status != "pass"]
    return Verdict(not bad, "; ".join(bad) or f"{len(runs)} runs pass")


def mutated_verdict(runs: dict[str, str], status: int | None) -> Verdict:
    """Caught when a run fails a step, none could not run or lost its result, and qa.sh failed."""

    if not runs:
        return Verdict(False, "the mutated tree ran nothing; does the patch still build?")
    broken = [f"{run} {state}" for run, state in runs.items() if state in ("error", "missing")]
    if broken:
        return Verdict(False, "; ".join(broken))
    failed = [run for run, state in runs.items() if state == "fail"]
    if not failed:
        return Verdict(False, f"every run still passes: {', '.join(runs)}")
    if status == 0:
        return Verdict(False, "runs failed but qa.sh exited 0")
    return Verdict(True, f"{len(failed)} of {len(runs)} runs fail: {', '.join(failed)}")


def git(*arguments: str, repository: Path = REPOSITORY) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(repository), *arguments], capture_output=True, text=True, check=False
    )


def patched_files(patch: Path) -> list[str]:
    return sorted({
        line[len("+++ b/"):].strip()
        for line in patch.read_text(encoding="utf-8").splitlines()
        if line.startswith("+++ b/")
    })


class Mutation:
    def __init__(self, name: str, repository: Path = REPOSITORY, mutations: Path | None = None) -> None:
        self.repository = repository
        self.patch = (mutations or MUTATIONS) / f"{name}.patch"
        if not self.patch.is_file():
            raise SystemExit(f"unknown mutation: {name}")
        self.files = patched_files(self.patch)
        if not self.files:
            raise SystemExit(f"{self.patch} names no files")

    def __enter__(self) -> Mutation:
        dirty = git("status", "--porcelain", "--", *self.files, repository=self.repository).stdout.strip()
        if dirty:
            raise SystemExit(f"{self.patch.name} touches files with uncommitted changes:\n{dirty}")
        try:
            applied = git("apply", "-p1", str(self.patch), repository=self.repository)
            if applied.returncode:
                raise SystemExit(f"{self.patch.name} does not apply:\n{applied.stderr}")
        except BaseException:
            # The files were clean, so restoring them undoes any partial apply.
            self.__exit__(None, None, None)
            raise
        return self

    def __exit__(self, *_exception) -> None:
        git("restore", "--worktree", "--", *self.files, repository=self.repository)
        left = git("status", "--porcelain", "--", *self.files, repository=self.repository).stdout.strip()
        if left:
            raise SystemExit(f"could not restore {self.files} after {self.patch.name}:\n{left}")


def run_qa(names: list[str], destination: Path) -> tuple[dict[str, str], int]:
    destination.mkdir(parents=True, exist_ok=True)
    log = destination / "qa.log"
    started = time.monotonic()
    with log.open("w", encoding="utf-8") as output:
        status = subprocess.run(
            [str(REPOSITORY / "scripts/qa.sh"), *names], cwd=REPOSITORY,
            stdout=output, stderr=subprocess.STDOUT, check=False,
        ).returncode
    print(f"   qa.sh exited {status} after {time.monotonic() - started:.0f}s; "
          f"log {log.relative_to(REPOSITORY)}", flush=True)
    return parse_qa_log(log.read_text(encoding="utf-8", errors="replace")), status


@dataclass
class Report:
    target: PatchTarget
    baseline: Verdict
    mutated: Verdict | None = None
    runs: dict[str, str] = field(default_factory=dict)

    @property
    def caught(self) -> bool:
        return self.baseline.caught and self.mutated is not None and self.mutated.caught


def write_summary(reports: list[Report], elapsed: float | None, output: Path | None = None) -> str:
    def cell(text: str) -> str:
        return text.replace("|", "\\|").replace("\n", " ") or "–"

    lines = ["| Patch | Scenario | Unmodified | Mutated |", "| --- | --- | --- | --- |"]
    for report in reports:
        mutated = report.mutated.note if report.mutated else "not run: the unmodified runs do not pass"
        lines.append(
            f"| {report.target.patch} | {report.target.scenario} | {cell(report.baseline.note)} | "
            f"{'caught' if report.caught else 'MISSED'}: {cell(mutated)} |"
        )
    took = f" ({elapsed / 60:.1f} minutes)" if elapsed is not None else ""
    lines += ["", f"{sum(report.caught for report in reports)} of {len(reports)} mutations caught{took}."]
    text = "\n".join(lines) + "\n"
    output = output or OUTPUT
    output.mkdir(parents=True, exist_ok=True)
    (output / "summary.md").write_text(text, encoding="utf-8")
    return text


def check(names: list[str]) -> int:
    started = time.monotonic()
    chosen = chosen_patches(names)
    (OUTPUT / "summary.md").unlink(missing_ok=True)
    if not chosen:
        print("No mutation patch under tests/qa/mutations.", file=sys.stderr)
        return 2
    unique_scenarios = list(dict.fromkeys(target.scenario for target in chosen))
    print("== unmodified tree", flush=True)
    baseline, _ = run_qa(unique_scenarios, OUTPUT / "baseline")
    reports = []
    for target in chosen:
        report = Report(target, baseline_verdict(own_runs(target.scenario, baseline)))
        reports.append(report)
        if not report.baseline.caught:
            print(f"== {target.patch}: skipped, {report.baseline.note}", flush=True)
            continue
        print(f"== {target.patch}: mutated tree", flush=True)
        with Mutation(target.patch):
            runs, status = run_qa([target.scenario], OUTPUT / "mutations" / target.patch)
        report.runs = own_runs(target.scenario, runs)
        report.mutated = mutated_verdict(report.runs, status)
    summary = write_summary(reports, time.monotonic() - started)
    print("\n" + summary)
    return 0 if all(report.caught for report in reports) else 1


def main(arguments: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "names",
        nargs="*",
        help="scenario names, <scenario>.patch, <scenario>.<label>, or patch paths; every patch if none",
    )
    options = parser.parse_args(arguments)

    def terminate(_number, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, terminate)
    try:
        return check(options.names)
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
