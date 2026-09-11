#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build once, partition libtest inventories, and fail closed on shard coverage."""

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

from e2e_bundle import image_key, source_key

ROOT = Path(__file__).resolve().parents[1]
BUNDLE = ROOT / "target/quality-bundle"
REPORTS = ROOT / "target/quality-reports"
SHARDS = 4
SUMMARY = re.compile(
    r"^test result: ok\. (\d+) passed; 0 failed; (\d+) ignored; 0 measured; \d+ filtered out;.*$",
    re.MULTILINE,
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(binary, filters=(), ignored=False):
    command = [str(binary), "--list", "--format=terse", "--exact", *filters]
    if ignored:
        command.append("--ignored")
    output = subprocess.check_output(command, text=True, cwd=ROOT)
    names = []
    for line in output.splitlines():
        if not line:
            continue
        if not line.endswith(": test"):
            raise ValueError(f"Unsupported libtest inventory entry: {line!r}")
        names.append(line.removesuffix(": test"))
    if len(names) != len(set(names)):
        raise ValueError("Duplicate libtest names")
    return sorted(names)


def partition(tests, durations):
    loads = [0.0] * SHARDS
    assignments = [[] for _ in loads]
    for test in sorted(tests, key=lambda name: (-durations.get(name, 1.0), name)):
        shard = min(range(SHARDS), key=lambda index: (loads[index], index))
        assignments[shard].append(test)
        loads[shard] += durations.get(test, 1.0)
    return [sorted(names) for names in assignments]


def test_source_key(repository):
    inputs = [source_key(repository)]
    for path in sorted((repository / "tests").rglob("*.rs")):
        inputs.extend((str(path.relative_to(repository)), digest(path)))
    return hashlib.sha256(json.dumps(inputs).encode()).hexdigest()


def build():
    BUNDLE.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        ["cargo", "test", "--locked", "--all-targets", "--all-features", "--no-run",
         "--message-format=json"], cwd=ROOT, text=True, stdout=subprocess.PIPE,
    )
    artifacts = {}
    for line in result.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-message":
            print(message["message"].get("rendered", ""), end="", flush=True)
        if message.get("reason") == "compiler-artifact" and message.get("profile", {}).get("test"):
            executable = message.get("executable")
            if executable:
                artifacts[executable] = message["target"]["name"]
    result.check_returncode()
    if not artifacts:
        raise ValueError("Cargo produced no test executables")
    durations = json.loads((ROOT / "scripts/quality-durations.json").read_text())
    binaries = []
    for index, (executable, target) in enumerate(sorted(artifacts.items())):
        destination = BUNDLE / f"test-{index}"
        shutil.copy2(executable, destination)
        tests = inventory(destination)
        ignored = inventory(destination, ignored=True)
        binaries.append(dict(file=destination.name, target=target, sha256=digest(destination),
                             tests=tests, ignored=ignored, shards=partition(tests, durations)))
    plan = dict(version=1, commit=os.environ["STRATA_QUALITY_COMMIT"],
                source_key=test_source_key(ROOT), image_key=image_key(ROOT), binaries=binaries)
    validate_plan(plan)
    (BUNDLE / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")


def validate_plan(plan):
    if plan["version"] != 1 or not plan["binaries"]:
        raise ValueError("Unsupported or empty plan")
    files = []
    totals = [0] * SHARDS
    for binary in plan["binaries"]:
        name = binary["file"]
        if not re.fullmatch(r"test-\d+", name):
            raise ValueError("Invalid binary path")
        files.append(name)
        tests, ignored, shards = binary["tests"], binary["ignored"], binary["shards"]
        if len(tests) != len(set(tests)) or len(ignored) != len(set(ignored)):
            raise ValueError("Duplicate inventory")
        if not set(ignored) <= set(tests) or len(shards) != SHARDS:
            raise ValueError("Invalid ignored inventory or shard count")
        if Counter(test for shard in shards for test in shard) != Counter(tests):
            raise ValueError("Missing, extra, or duplicate assignment")
        for index, shard in enumerate(shards):
            totals[index] += len(set(shard) - set(ignored))
    if len(files) != len(set(files)) or not all(totals):
        raise ValueError("Duplicate binary or empty runnable shard")


def run(shard):
    if shard not in range(SHARDS):
        raise ValueError("Invalid shard")
    REPORTS.mkdir(parents=True, exist_ok=True)
    report_path = REPORTS / f"shard-{shard}.json"
    report_path.unlink(missing_ok=True)
    plan_path = BUNDLE / "plan.json"
    plan = json.loads(plan_path.read_text())
    validate_plan(plan)
    if (plan["commit"] != os.environ["STRATA_QUALITY_COMMIT"]
            or plan["source_key"] != test_source_key(ROOT) or plan["image_key"] != image_key(ROOT)):
        raise ValueError("Bundle checkout or environment mismatch")
    started = time.monotonic()
    report = dict(plan_sha256=digest(plan_path), shard=shard, binaries=[])
    for binary in plan["binaries"]:
        executable = BUNDLE / binary["file"]
        if digest(executable) != binary["sha256"]:
            raise ValueError("Binary checksum mismatch")
        executable.chmod(0o755)
        if inventory(executable) != binary["tests"] or inventory(executable, ignored=True) != binary["ignored"]:
            raise ValueError("Binary inventory mismatch")
        selected = binary["shards"][shard]
        if not selected:
            continue
        if inventory(executable, selected) != selected:
            raise ValueError("Libtest selection mismatch")
        command = [str(executable), "--exact", *selected, "--nocapture"]
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT)
        log = result.stdout
        print(log, end="", flush=True)
        (REPORTS / f"shard-{shard}-{binary['file']}.log").write_text(log)
        summaries = SUMMARY.findall(log)
        ignored = sorted(set(selected) & set(binary["ignored"]))
        expected = (str(len(selected) - len(ignored)), str(len(ignored)))
        if result.returncode or not summaries or summaries[-1] != expected:
            raise ValueError(f"{binary['file']} failed or did not execute its complete selection")
        report["binaries"].append(dict(file=binary["file"], passed=sorted(set(selected) - set(ignored)),
                                       ignored=ignored))
    report["seconds"] = round(time.monotonic() - started, 2)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Shard {shard} completed in {report['seconds']}s", flush=True)


def verify(plan_path, reports):
    plan = json.loads(plan_path.read_text())
    validate_plan(plan)
    paths = sorted(reports.glob("shard-*.json"))
    if len(paths) != SHARDS:
        raise ValueError("Missing or extra shard reports")
    seen = set()
    for path in paths:
        report = json.loads(path.read_text())
        shard = report["shard"]
        if type(shard) is not int or shard not in range(SHARDS) or shard in seen:
            raise ValueError("Invalid or duplicate shard report")
        seen.add(shard)
        if report["plan_sha256"] != digest(plan_path):
            raise ValueError("Stale shard report")
        expected = []
        for binary in plan["binaries"]:
            selected = set(binary["shards"][shard])
            if selected:
                ignored = selected & set(binary["ignored"])
                expected.append(dict(file=binary["file"], passed=sorted(selected - ignored),
                                     ignored=sorted(ignored)))
        if report["binaries"] != expected:
            raise ValueError("Missing, extra, failed, or duplicate test result")
    total = sum(len(binary["tests"]) - len(binary["ignored"]) for binary in plan["binaries"])
    ignored = sum(len(binary["ignored"]) for binary in plan["binaries"])
    print(f"Verified {total} passed tests exactly once; {ignored} explicitly ignored by libtest")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "run", "verify"))
    parser.add_argument("--shard", type=int, default=os.environ.get("STRATA_QUALITY_SHARD"))
    parser.add_argument("--plan", type=Path, default=BUNDLE / "plan.json")
    parser.add_argument("--reports", type=Path, default=REPORTS)
    args = parser.parse_args()
    if args.command == "build":
        build()
    elif args.command == "run":
        run(args.shard)
    else:
        verify(args.plan, args.reports)


if __name__ == "__main__":
    main()
