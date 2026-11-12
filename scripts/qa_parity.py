#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reads the parity ledger for scripts/qa-parity.sh: the runs a selection names, their
counts by status and reason, and the failures of the whole GTK suite its allowances let
`slice` pass over.

The ledger is the directory tests/qa/parity: <folder>.toml lists the runs of one folder
under tests/qa/scenarios, and slice.toml holds the slice's allowances (docs/testing.md).
It is checked before anything is selected from it, by scripts/qa_parity_check.py and, for
the labelled twins, scripts/qa_parity_twins.py: a ledger the runner's tests would refuse
selects nothing.

A `parallel` run is at parity as a `required` one is, by another route: its scenario passes
on GTK, and its twin, `<scenario>-rune/<run>`, on Rune. So the runs that must pass differ by
target, and `--on` names the target. `--twins` lists the twins for a review or a checkpoint,
and `--status parallel` names the runs whose shared file a survey tries on Rune again.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from collections import Counter
from pathlib import Path

import qa_parity_check
import qa_parity_twins
from qa_parity_check import EXCEPTIONS, HOSTS, STATUSES

REPOSITORY = Path(__file__).resolve().parents[1]
LEDGER = REPOSITORY / "tests/qa/parity"
SCENARIOS = REPOSITORY / "tests/qa/scenarios"
PACKET = REPOSITORY / qa_parity_check.PACKET
# The Rune mutation patches, in the ledger's directory: each is named for a scenario.
MUTATIONS = "mutations"
# The statuses of a run at parity, which a selection runs: the shared file on both targets,
# or the scenario on GTK and its twin on Rune.
AT_PARITY = ("required", "parallel")
TARGETS = ("gtk", "rune")
# The one file of the directory that lists no runs: the slice's allowances.
ALLOWANCES = "slice.toml"
GATES = {"g1": ("G1",), "g2": ("G2",), "g3": ("G3",), "slice": ("G1", "G2", "G3")}
EVERY = "all"
MODES = ("columns", "icons", "list")
# The host this command can start. A `session` run needs a real window in a compositor.
OFFSCREEN = "offscreen"
# The runner's line for a failed test and its closing count of them, tests/qa/harness.rs.
FAILED = re.compile(r"^test (\S+) \.\.\. FAILED\b")
RESULT = re.compile(r"^test result: FAILED\. \d+ passed; (\d+) failed;")


class UnknownSelection(ValueError):
    """A selection that is no gate, folder or package of the ledger."""


class BadLedger(ValueError):
    """A ledger the runner's tests would refuse; `problems` has one line for each fault."""

    def __init__(self, problems: list[str]):
        super().__init__("\n".join(problems))
        self.problems = problems


def name(run: dict) -> str:
    return f"{run['scenario']}/{run['run']}"


def folders(ledger: dict) -> list[str]:
    return sorted({run["folder"] for run in ledger.get("run", [])})


def packages(ledger: dict) -> list[str]:
    return sorted({run["package"] for run in ledger.get("run", [])}, key=package_order)


def package_order(package: str) -> tuple[str, int, str]:
    """Packages by track and then number: A2 before A10."""

    track, number = package[:1], package[1:]
    return (track, int(number), "") if number.isdigit() else (package, 0, "")


def chosen(ledger: dict, selection: str) -> list[dict]:
    """The runs a selection names, in ledger order: a gate's, a folder's, a package's or all.

    `slice` and the gates are the slice's set by its `gate` field, whoever owns the run.
    A folder or a package is every view's runs.
    """

    runs = ledger.get("run", [])
    if selection == EVERY:
        return list(runs)
    if selection in GATES:
        return [run for run in runs if run.get("gate") in GATES[selection]]
    for field, known in (("folder", folders(ledger)), ("package", packages(ledger))):
        if selection in known:
            return [run for run in runs if run[field] == selection]
    raise UnknownSelection(
        f"{selection!r} is no selection: it is {', '.join(GATES)} or {EVERY}, a folder of "
        f"tests/qa/scenarios, or a package that owns a run ({', '.join(packages(ledger))})")


def twin_scenario(run: dict) -> str | None:
    """The scenario of the twin that stands in for a parallel run."""

    return run["twin"].split("/", 1)[0] if run["status"] == "parallel" else None


def named(run: dict, scenario: str | None) -> bool:
    """Whether a scenario's name picks the run: its own, or its twin's, which picks the pair."""

    return scenario in (None, run["scenario"], twin_scenario(run))


def selected(ledger: dict, selection: str, status: str, scenario: str | None = None) -> list[str]:
    """The `<scenario>/<run>` names of a selection's runs with the status, in ledger order.

    With `scenario`, only that scenario's runs, or the runs that twin stands in for.
    """

    return [name(run) for run in chosen(ledger, selection)
            if run["status"] == status and named(run, scenario)]


def deciding(ledger: dict, selection: str, target: str, scenario: str | None = None) -> list[str]:
    """What must pass on a target for the selection's runs at parity, in ledger order: each
    required run, and for each parallel run its scenario on GTK and its twin on Rune."""

    on_rune = target == "rune"
    return [run["twin"] if on_rune and run["status"] == "parallel" else name(run)
            for run in chosen(ledger, selection)
            if run["status"] in AT_PARITY and named(run, scenario)]


def needs_session(ledger: dict, selection: str, scenario: str | None = None) -> list[str]:
    """The selection's runs at parity this command cannot start: the session host's. A
    required run names its host; a parallel one names it only when its twin needs it."""

    implied = {"required": None, "parallel": OFFSCREEN}
    return [name(run) for run in chosen(ledger, selection)
            if run["status"] in AT_PARITY and named(run, scenario)
            and run.get("host", implied[run["status"]]) != OFFSCREEN]


def mode(run: dict) -> str:
    """The view a run starts in: its label is the mode, or the mode and a variant."""

    return run["run"].split("-", 1)[0]


def counts(ledger: dict, selection: str) -> str:
    """A selection's runs by status, reason and exception, by host and by view, its twins by
    exception, and its runs by package."""

    runs = chosen(ledger, selection)
    spread = len({run["folder"] for run in runs})
    # A parallel run that names no host is on the offscreen one.
    implied = {"parallel": OFFSCREEN}
    by_host = Counter(host for run in runs if (host := run.get("host", implied.get(run["status"]))))
    by_mode = Counter(mode(run) for run in runs)
    return "\n".join([
        f"== {selection}: {len(runs)} run(s) in {spread} folder(s) ==",
        *_by_status(runs),
        "host      " + _listed(by_host, HOSTS),
        "view      " + _listed(by_mode, MODES),
        "twins     " + _listed(_twins(runs), EXCEPTIONS),
        *_by_package(runs),
    ])


def _by_status(runs: list[dict]) -> list[str]:
    """A line for each status, with the parallel runs' exceptions and the pending runs'
    reasons under theirs."""

    by_status = Counter(run["status"] for run in runs)
    under = {status: Counter(run[field] for run in runs if run["status"] == status)
             for status, field in (("parallel", "exception"), ("pending", "reason"))}
    return [line for status in STATUSES
            for line in (f"{status:<10}{by_status[status]:>5}",
                         *(f"  {why:<10}{count:>5}" for why, count in sorted(under.get(status, {}).items())))]


def _twins(runs: list[dict]) -> Counter:
    """The twin files that stand in for the runs, by exception: a twin with two runs is one."""

    twins = {(run["exception"], twin_scenario(run)) for run in runs if run["status"] == "parallel"}
    return Counter(exception for exception, _ in twins)


def _by_package(runs: list[dict]) -> list[str]:
    """A heading, then a line for each package: its runs, and how many have each status."""

    lines = [f"{'package':<10}{'runs':>5}" + "".join(f"{status:>10}" for status in STATUSES)]
    for package in packages({"run": runs}):
        owned = Counter(run["status"] for run in runs if run["package"] == package)
        lines.append(f"{package:<10}{sum(owned.values()):>5}"
                     + "".join(f"{owned[status]:>10}" for status in STATUSES))
    return lines


def twin_listing(ledger: dict, selection: str) -> str:
    """A selection's twins, each with what a reviewer or a checkpoint audits: the exception
    and its reference, the runs it stands in for, what its header says differs, and how its
    steps stand to its original's over those runs."""

    runs = [run for run in chosen(ledger, selection) if run["status"] == "parallel"]
    names = list(dict.fromkeys(twin_scenario(run) for run in runs))
    lines = [f"== {selection}: {len(names)} twin(s) for {len(runs)} parallel run(s) =="]
    for twin in names:
        own = [run for run in runs if twin_scenario(run) == twin]
        kept, added, lost = ledger["twins"][twin]["steps"]
        lines += [
            f"{twin}  {own[0]['exception']}  {own[0]['ref']}  {own[0]['package']}",
            f"  stands in for  {', '.join(name(run) for run in own)}",
            f"  differs        {ledger['twins'][twin]['differs']}",
            f"  steps          {kept} the original's, {added} its own, {lost} of the original's dropped",
        ]
    return "\n".join(lines)


def _listed(counter: Counter, order: tuple[str, ...]) -> str:
    return ", ".join(f"{key} {counter[key]}" for key in order)


def flaky(ledger: dict) -> list[str]:
    """The `<scenario>/<run>` names the ledger allows to fail in the whole GTK suite."""

    return [name(run) for run in ledger.get("flaky", [])]


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
    return ([test for test in failed if test in allowed],
            [test for test in failed if test not in allowed])


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
    for run in excused(ledger, text)[0]:
        print(run)
    return 0


def read(path: Path) -> dict:
    with path.open("rb") as stream:
        return tomllib.load(stream)


def load(ledger: Path = LEDGER, scenarios: Path = SCENARIOS, packet: Path = PACKET) -> dict:
    """Every folder's runs, each with its `folder`, in file and then ledger order; and the
    allowances, when the directory has them.

    The ledger is checked first against the runs the scenario files under `scenarios` plan,
    their `parity` labels, the Rune mutation patches beside the ledger and the entries of
    the filing packet; one with a fault is refused whole.
    """

    files = sorted(path for path in ledger.glob("*.toml") if path.name != ALLOWANCES)
    texts = {path.stem: path.read_text(encoding="utf-8") for path in files}
    corpus = qa_parity_twins.corpus(scenarios, ledger / MUTATIONS, packet)
    if problems := qa_parity_twins.faults(texts, corpus):
        raise BadLedger(problems)
    runs = [{**run, "folder": folder} for folder, text in texts.items()
            for run in tomllib.loads(text).get("run", [])]
    allowances = read(ledger / ALLOWANCES) if (ledger / ALLOWANCES).is_file() else {}
    return {"run": runs, "twins": qa_parity_twins.twins(corpus),
            **{key: allowances.get(key, []) for key in ("flaky", "stale_patch")}}


def refuse(ledger: Path, error: BadLedger) -> int:
    """Says what is wrong with the ledger and what to do about an unlisted run; exits 2."""

    print(f"{display(ledger)}, or a pair it names, is not in order, so nothing is selected from it:",
          file=sys.stderr)
    for problem in error.problems:
        print(f"  {problem}", file=sys.stderr)
    if any(problem.endswith("is not listed") for problem in error.problems):
        print(qa_parity_check.HELP, file=sys.stderr)
    return 2


def display(path: Path) -> str:
    return str(path.relative_to(REPOSITORY)) if path.is_relative_to(REPOSITORY) else str(path)


def report_selected(ledger: dict, args: argparse.Namespace) -> int:
    """Prints a selection's runs, one a line: with `--on`, what must pass on that target,
    and otherwise the runs with the status. Exits 3 when a run at parity needs a host this
    command cannot start."""

    at_parity = args.on is not None or args.status in AT_PARITY
    if at_parity and (beyond := needs_session(ledger, args.selection, args.scenario)):
        print(f"{len(beyond)} required or parallel run(s) of {args.selection} name the session "
              f"host, which qa-parity.sh cannot start yet: {', '.join(beyond)}", file=sys.stderr)
        return 3
    runs = (deciding(ledger, args.selection, args.on, args.scenario) if args.on
            else selected(ledger, args.selection, args.status, args.scenario))
    for run in runs:
        print(run)
    return 0


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("selection", help=f"{', '.join(GATES)}, {EVERY}, a scenario folder or a package")
    parser.add_argument("--status", choices=STATUSES, default="required")
    parser.add_argument("--on", choices=TARGETS,
                        help="instead of the runs with a status: what must pass on this target, "
                             "the required runs and, for each parallel run, its scenario on gtk "
                             "and its twin on rune")
    parser.add_argument("--scenario", help="only this scenario's runs, or a twin's pair")
    parser.add_argument("--counts", action="store_true",
                        help="instead of the runs: the selection's counts by status and reason")
    parser.add_argument("--twins", action="store_true",
                        help="instead of the runs: the selection's twins, each with its exception, "
                             "its reference, what differs and how its steps stand to its original's")
    parser.add_argument("--ledger", type=Path, default=LEDGER, help="the ledger's directory")
    parser.add_argument("--scenarios", type=Path, default=SCENARIOS,
                        help="the scenario folders whose runs the ledger lists")
    parser.add_argument("--packet", type=Path, default=PACKET,
                        help="the Rune filing packet, whose entries a rune twin cites")
    parser.add_argument("--failed-in", type=Path, metavar="LOG",
                        help="instead of a selection's runs: the flaky runs that failed in this "
                             "log of the whole GTK suite, or exit 1 if it failed any other way")
    args = parser.parse_args(arguments)
    try:
        ledger = load(args.ledger, args.scenarios, args.packet)
    except BadLedger as error:
        return refuse(args.ledger, error)
    if args.failed_in:
        return report_excused(ledger, args.failed_in)
    try:
        if args.counts or args.twins:
            print(counts(ledger, args.selection) if args.counts else twin_listing(ledger, args.selection))
            return 0
        return report_selected(ledger, args)
    except UnknownSelection as error:
        print(error, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
