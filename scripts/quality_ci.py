#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build once, partition libtest inventories, and fail closed on shard coverage."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

from image_inputs import image_key, source_key

ROOT = Path(__file__).resolve().parents[1]
BUNDLE = ROOT / "target/quality-bundle"
REPORTS = ROOT / "target/quality-reports"
LOCAL = ROOT / "target/quality-local"
SHARDS = 4
# Tests outside GTK children share libtest's threads, four on CI runners, instead of queueing
# for the display, so they cost a shard about a quarter of their serial time.
PARALLEL_DISCOUNT = 4
# GTK tests serialize per display, not per machine, so each lane of a shard owns an Xvfb server.
FIRST_DISPLAY = 99
XVFB_SERVER = "-screen 0 1280x1024x24 -nolisten tcp -noreset"
PRIVATE_GTK_ENV = ("GDK_BACKEND=x11", "GTK_A11Y=none", "NO_AT_BRIDGE=1", "STRATA_REQUIRE_GTK_TESTS=1")
ISOLATED_TEST = (
    "ui::search::tests::"
    "deferred_scroll_restoration_yields_to_updates_wheel_scrollbar_and_query_reset"
)
# Parent libtest lines start a test; a GTK child repeats its name and prints no time of its own.
TIMED = re.compile(r"^test (\S+) \.\.\. |\bok <(\d+(?:\.\d+)?)s>$", re.MULTILINE)
TIMING_ARGS = ("-Z", "unstable-options", "--report-time", "--test-threads=1")
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


def partition(tests, durations, count=None):
    count = SHARDS if count is None else count
    loads = [0.0] * count
    assignments = [[] for _ in loads]
    for test in sorted(tests, key=lambda name: (-durations.get(name, 1.0), name)):
        shard = min(range(count), key=lambda index: (loads[index], index))
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
    durations = balancing_hints()
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
    if sum(ISOLATED_TEST in binary["tests"] and ISOLATED_TEST not in binary["ignored"]
           for binary in plan["binaries"]) != 1:
        raise ValueError("Expected exactly one runnable isolated test")
    if len(files) != len(set(files)) or not all(totals):
        raise ValueError("Duplicate binary or empty runnable shard")


def test_times(output):
    """Each test's serial wall time, and whether it ran in an isolated GTK child."""
    times, starts, current = {}, Counter(), None
    for match in TIMED.finditer(output):
        if match[1]:
            current = match[1]
            starts[current] += 1
        elif current is not None:
            times[current] = float(match[2])
            current = None
    return {name: dict(seconds=seconds, gtk=starts[name] > 1) for name, seconds in times.items()}


def balancing_weights(measured):
    """Seconds each test occupies its shard; zero weights would all land on one shard."""
    return {name: max(0.01, round(time["seconds"] / (1 if time["gtk"] else PARALLEL_DISCOUNT), 2))
            for name, time in sorted(measured.items())}


def balancing_hints():
    return json.loads((ROOT / "scripts/quality-durations.json").read_text())


def default_lanes():
    return max(1, len(os.sched_getaffinity(0)) // 2)


def private_display(display, command):
    return ["xvfb-run", "-n", str(display), "-s", XVFB_SERVER, "dbus-run-session", "--",
            "env", "-u", "WAYLAND_DISPLAY", *PRIVATE_GTK_ENV, *command]


def run_lane(lane, lanes, work, timings, log_path):
    """Run one lane's tests on displays no other lane uses; libtest must account for each batch."""
    started = time.monotonic()
    displays = iter(range(FIRST_DISPLAY + lane, sys.maxsize, lanes))
    measured = {}
    with log_path.open("wb") as log:
        for binary, executable, tests in work:
            batches = [[test for test in tests if test == ISOLATED_TEST],
                       [test for test in tests if test != ISOLATED_TEST]]
            for batch in filter(None, batches):
                command = [str(executable), "--exact", *batch, "--nocapture"]
                environment = None
                if timings:
                    # Stable libtest reports times only behind -Z; one thread keeps GTK lock waits out.
                    command.extend(TIMING_ARGS)
                    environment = {**os.environ, "RUSTC_BOOTSTRAP": "1"}
                log.flush()
                offset = log.tell()
                code = subprocess.run(private_display(next(displays), command), cwd=ROOT,
                                      stdout=log, stderr=subprocess.STDOUT, env=environment).returncode
                log.seek(0, os.SEEK_END)
                with log_path.open("rb") as written:
                    written.seek(offset)
                    output = written.read().decode(errors="replace")
                summaries = SUMMARY.findall(output)
                ignored_count = len(set(batch) & set(binary["ignored"]))
                expected = (str(len(batch) - ignored_count), str(ignored_count))
                if code or not summaries or summaries[-1] != expected:
                    log.write(f"\n{binary['file']} failed or did not execute its complete selection\n".encode())
                    return lane, False, time.monotonic() - started, measured
                measured.update(test_times(output))
    return lane, True, time.monotonic() - started, measured


def check_lanes(lanes):
    if type(lanes) is not int or lanes < 1:
        raise ValueError("Lane count must be a positive integer")


def run(shard, lanes=1, timings=False):
    if shard not in range(SHARDS):
        raise ValueError("Invalid shard")
    check_lanes(lanes)
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
    hints = balancing_hints()
    work = [[] for _ in range(lanes)]
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
        for lane, tests in enumerate(partition(selected, hints, lanes)):
            if tests:
                work[lane].append((binary, executable, tests))
        ignored = sorted(set(selected) & set(binary["ignored"]))
        report["binaries"].append(dict(file=binary["file"], passed=sorted(set(selected) - set(ignored)),
                                       ignored=ignored))
    # Lane output goes only to the logs, so a hung lane is diagnosed from them, not this console.
    logs = (REPORTS / f"shard-{shard}-lane-*.log").relative_to(ROOT)
    print(f"Shard {shard}: running {lanes} lanes, each on its own display and session bus; "
          f"output in {logs}", flush=True)
    failed, measured = [], {}
    with ThreadPoolExecutor(lanes) as pool:
        futures = [pool.submit(run_lane, lane, lanes, tests, timings, REPORTS / f"shard-{shard}-lane-{lane}.log")
                   for lane, tests in enumerate(work) if tests]
        for future in as_completed(futures):
            lane, passed, seconds, times = future.result()
            print(f"Shard {shard} lane {lane} {'passed' if passed else 'FAILED'} in {seconds:.1f}s", flush=True)
            measured.update(times)
            if not passed:
                failed.append(lane)
    for lane in sorted(failed):
        log = REPORTS / f"shard-{shard}-lane-{lane}.log"
        print(f"\n===== shard {shard} lane {lane} failure excerpt; full log: {log} =====")
        print(failure_excerpt(log.read_text(errors="replace")), flush=True)
    if failed:
        raise ValueError(f"{len(failed)} of {lanes} lanes failed in shard {shard}")
    report["seconds"] = round(time.monotonic() - started, 2)
    if timings:
        (REPORTS / f"timings-{shard}.json").write_text(json.dumps(measured, indent=2) + "\n")
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


def failure_excerpt(output):
    """Panics from the failed tests, skipping the expected panics passing tests print."""
    lines = output.splitlines()
    failed = set(re.findall(r"^test (\S+) \.\.\. FAILED$", output, re.MULTILINE))
    excerpt = []
    for index, line in enumerate(lines):
        thread = re.match(r"thread '([^']+)'.* panicked at ", line)
        if thread and (not failed or thread[1] in failed) and len(excerpt) < 60:
            excerpt.extend(lines[index:index + 3])
    return "\n".join([*excerpt, "...", *lines[-30:]])


def use_local_bundle():
    global BUNDLE, REPORTS, SHARDS
    BUNDLE, REPORTS, SHARDS = LOCAL / "bundle", LOCAL / "reports", 1


def local(lanes, timings=False):
    """Run the complete suite as one shard whose lanes share this machine."""
    check_lanes(lanes)
    use_local_bundle()
    started = time.monotonic()
    build()
    shutil.rmtree(REPORTS, ignore_errors=True)
    run(0, lanes, timings)
    verify(BUNDLE / "plan.json", REPORTS)
    if timings:
        measured = json.loads((REPORTS / "timings-0.json").read_text())
        runnable = {test for binary in json.loads((BUNDLE / "plan.json").read_text())["binaries"]
                    for test in set(binary["tests"]) - set(binary["ignored"])}
        if set(measured) != runnable:
            raise ValueError("Timings do not cover exactly the runnable tests")
        path = ROOT / "scripts/quality-durations.json"
        path.write_text(json.dumps(balancing_weights(measured), indent=2) + "\n")
        print(f"Wrote balancing weights for {len(measured)} tests to {path.relative_to(ROOT)}")
    print(f"Rust tests finished in {time.monotonic() - started:.1f}s", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "run", "verify", "local"))
    parser.add_argument("--shard", type=int, default=os.environ.get("STRATA_QUALITY_SHARD"))
    parser.add_argument("--plan", type=Path, default=BUNDLE / "plan.json")
    parser.add_argument("--reports", type=Path, default=REPORTS)
    parser.add_argument("--lanes", type=int, default=None,
                        help="concurrent displays on this machine (default: half the available CPUs)")
    parser.add_argument("--refresh-timings", action="store_true",
                        help="local only: measure every test and rewrite scripts/quality-durations.json")
    args = parser.parse_args()
    lanes = default_lanes() if args.lanes is None else args.lanes
    if args.refresh_timings and args.command != "local":
        parser.error("--refresh-timings is only for the local command")
    if args.command == "build":
        build()
    elif args.command == "run":
        run(args.shard, lanes)
    elif args.command == "local":
        local(lanes, args.refresh_timings)
    else:
        verify(args.plan, args.reports)


if __name__ == "__main__":
    main()
