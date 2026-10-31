#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Measures each `--qa` scenario run and writes its relative cost into the scenario's header."""

from __future__ import annotations

import argparse
import json
import re
import statistics
import subprocess
import sys
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]
SCENARIOS = REPOSITORY / "tests/qa/scenarios"
OUTPUT = REPOSITORY / "target/qa-cost"
MINIMUM = 0.1
RESULT = re.compile(
    r"^test (?P<run>[^\s/]+/\S+) \.\.\. (?P<status>ok|FAILED) <(?P<seconds>\d+(?:\.\d+)?)s>$"
)
SPACE = re.compile(r"\s*")


def run_labels(header: dict) -> list[str]:
    """The run labels `tests/qa.rs` gives a scenario's runs: the mode, plus `-<variant>`."""

    variants = header.get("variants") or []
    if not variants:
        return list(header.get("modes", []))
    return [
        f"{mode}-{variant['id']}"
        for variant in variants
        for mode in (variant.get("modes") or header.get("modes", []))
    ]


def durations(log: str) -> dict[str, tuple[float, bool]]:
    found = {}
    for line in log.splitlines():
        if match := RESULT.match(line.rstrip()):
            found[match["run"]] = (float(match["seconds"]), match["status"] == "ok")
    return found


def scores(samples: dict[str, list[float]]) -> dict[str, float]:
    medians = {run: statistics.median(seconds) for run, seconds in samples.items()}
    typical = statistics.median(medians.values())
    if typical <= 0:
        raise ValueError("the typical run took no time")
    return {run: max(MINIMUM, round(median / typical, 1)) for run, median in medians.items()}


def value_spans(header: str) -> tuple[dict[str, tuple[int, int]], int]:
    decoder = json.JSONDecoder()
    spans = {}
    index = SPACE.match(header, 0).end()
    if header[index:index + 1] != "{":
        raise ValueError("the header is not a JSON object")
    index = SPACE.match(header, index + 1).end()
    if header[index:index + 1] == "}":
        return spans, index
    while True:
        key, index = decoder.raw_decode(header, index)
        index = SPACE.match(header, index).end()
        if header[index:index + 1] != ":":
            raise ValueError("the header is not a JSON object")
        start = SPACE.match(header, index + 1).end()
        _, end = decoder.raw_decode(header, start)
        spans[key] = (start, end)
        index = SPACE.match(header, end).end()
        if header[index:index + 1] == "}":
            return spans, index
        if header[index:index + 1] != ",":
            raise ValueError("the header is not a JSON object")
        index = SPACE.match(header, index + 1).end()


def with_cost(header: str, cost: dict[str, float]) -> str:
    text = json.dumps(cost, separators=(",", ":"))
    spans, close = value_spans(header)
    if "cost" in spans:
        start, end = spans["cost"]
        updated = header[:start] + text + header[end:]
    else:
        updated = header[:close] + ("," if spans else "") + '"cost":' + text + header[close:]
    if json.loads(updated) != {**json.loads(header), "cost": cost}:
        raise ValueError("rewriting the cost changed something else")
    return updated


def rewrite(path: Path, cost: dict[str, float]) -> bool:
    data = path.read_bytes()
    first, newline, rest = data.partition(b"\n")
    header = first.decode("utf-8")
    updated = with_cost(header, cost)
    if updated == header:
        return False
    path.write_bytes(updated.encode("utf-8") + newline + rest)
    return True


def planned(scenarios: Path = SCENARIOS) -> dict[Path, list[str]]:
    """Each scenario file's run labels, as `tests/qa.rs` names them."""

    found = {}
    for path in sorted(scenarios.glob("*/*.jsonl")):
        header = json.loads(path.read_text(encoding="utf-8").splitlines()[0])
        found[path] = run_labels(header)
    return found


def write(logs: list[Path], scenarios: Path = SCENARIOS) -> int:
    samples: dict[str, list[float]] = {}
    failed = set()
    for log in logs:
        for run, (seconds, passed) in durations(log.read_text(encoding="utf-8", errors="replace")).items():
            samples.setdefault(run, []).append(seconds)
            if not passed:
                failed.add(run)
    files = planned(scenarios)
    wanted = {f"{path.stem}/{label}" for path, labels in files.items() for label in labels}
    missing = sorted(wanted - samples.keys())
    if missing:
        print(f"no time for {len(missing)} runs, so no cost was written: {', '.join(missing)}",
              file=sys.stderr)
        return 1
    unknown = sorted(samples.keys() - wanted)
    if unknown:
        print(f"ignored, no longer planned: {', '.join(unknown)}", file=sys.stderr)
    if failed:
        print(f"timed although they failed: {', '.join(sorted(failed))}", file=sys.stderr)
    cost = scores({run: samples[run] for run in wanted})
    changed = sum(
        rewrite(path, {label: cost[f"{path.stem}/{label}"] for label in labels})
        for path, labels in files.items()
    )
    print(f"{changed} of {len(files)} scenario files changed; {len(wanted)} runs from {len(logs)} logs")
    print("costliest runs:")
    for run in sorted(cost, key=lambda run: (-cost[run], run))[:15]:
        print(f"  {cost[run]:5.1f}  {run}")
    return 0


def measure(repeat: int) -> list[Path]:
    """Full qa.sh runs, every scenario named so a skipped one is timed too."""

    names = [path.stem for path in planned()]
    OUTPUT.mkdir(parents=True, exist_ok=True)
    logs = []
    for number in range(1, repeat + 1):
        log = OUTPUT / f"run-{number}.log"
        print(f"== qa.sh run {number} of {repeat}; log {log.relative_to(REPOSITORY)}", flush=True)
        with log.open("w", encoding="utf-8") as output:
            status = subprocess.run(
                [str(REPOSITORY / "scripts/qa.sh"), *names],
                cwd=REPOSITORY, stdout=output, stderr=subprocess.STDOUT, check=False,
            ).returncode
        if status:
            print(f"   qa.sh exited {status}; the runs that finished still count", flush=True)
        logs.append(log)
    return logs


def main(arguments: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run", help="time full qa.sh runs, then write the costs")
    run.add_argument("--repeat", type=int, default=3, help="how many full runs (default 3)")
    saved = commands.add_parser("write", help="write the costs from saved qa.sh logs")
    saved.add_argument("logs", nargs="+", type=Path)
    options = parser.parse_args(arguments)
    if options.command == "write":
        return write(options.logs)
    if options.repeat < 1:
        parser.error("--repeat must be at least 1")
    return write(measure(options.repeat))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
