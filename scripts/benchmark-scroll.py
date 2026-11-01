#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Measure what scrolling costs in each browser mode."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import statistics
import struct
import subprocess
import sys
import time
import zlib
from dataclasses import dataclass, field
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]

from headless.display import HeadlessDisplay  # noqa: E402
from headless.environment import TestEnvironment, process_environment  # noqa: E402
from headless.process import ManagedProcess, terminate  # noqa: E402
from headless.xtest import XTestConnection  # noqa: E402

MODES = ("columns", "list", "icons")
FIXTURES = ("mixed", "images", "folders", "clipboard")
INPUTS = ("wheel", "keys")
FIXTURE_VERSION = 2
FIXTURE_FOLDERS = ("mixed", "images", "folders", "clip-source")
CLOCK_TICKS = os.sysconf("SC_CLK_TCK")
FRAME_BUDGET_NS = 16_666_667
WINDOW_WIDTH = 1280
WINDOW_HEIGHT = 800
CONTROL = 0xFFE3
SHIFT = 0xFFE1
RETURN = 0xFF0D
DOWN = 0xFF54
UP = 0xFF52
WHEEL_UP = 4
WHEEL_DOWN = 5

POINTER = {"columns": (420, 420), "list": (700, 420), "icons": (700, 420)}

MIXED_EXTENSIONS = (
    "rs", "py", "ts", "js", "json", "toml", "yaml", "md", "txt", "log", "sh", "c",
    "go", "html", "css", "pdf", "zip", "mp3", "png", "csv", "lock", "ini", "java", "rb",
)

TOOLS = {
    "Xvfb": "Xvfb (xorg-server-xvfb)",
    "sysprof-cli": "sysprof",
}


def png(width: int, height: int, seed: int) -> bytes:
    def chunk(kind: bytes, body: bytes) -> bytes:
        crc = zlib.crc32(kind + body) & 0xFFFFFFFF
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", crc)

    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows += bytes(
                ((x * 7 + seed * 13) & 0xFF, (y * 5 + seed * 29) & 0xFF, (x * y + seed) & 0xFF)
            )
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(rows), 6))
        + chunk(b"IEND", b"")
    )


def ensure_fixtures(root: Path) -> None:
    marker = root / f".complete-v{FIXTURE_VERSION}"
    if marker.exists():
        return
    for folder in FIXTURE_FOLDERS:
        shutil.rmtree(root / folder, ignore_errors=True)
    mixed = root / "mixed"
    mixed.mkdir(parents=True)
    for index in range(3000):
        if index % 50 == 0:
            (mixed / f"folder-{index:05d}").mkdir()
            continue
        extension = MIXED_EXTENSIONS[index % len(MIXED_EXTENSIONS)]
        stem = "entry" if index % 3 else "a-somewhat-longer-file-name-for-ellipsis"
        (mixed / f"{stem}-{index:05d}.{extension}").touch()
    images = root / "images"
    images.mkdir()
    for index in range(1200):
        (images / f"image-{index:05d}.png").write_bytes(png(96, 96, index))
    folders = root / "folders"
    folders.mkdir()
    for index in range(600):
        folder = folders / f"folder-{index:04d}"
        folder.mkdir()
        for child in range(index % 13):
            (folder / f"child-{child:02d}.txt").touch()
    source = root / "clip-source"
    source.mkdir()
    for index in range(1000):
        (source / f"copied-{index:05d}.txt").touch()
    marker.touch()


CAPTURE_MAGIC = 0xFDCA975E
CAPTURE_HEADER_SIZE = 256
CAPTURE_FRAME = struct.Struct("<HhiqII")
CAPTURE_MARK = struct.Struct("<q24s40s")
CAPTURE_MARK_KIND = 10


def read_marks(path: Path):
    data = path.read_bytes()
    if len(data) < CAPTURE_HEADER_SIZE or struct.unpack_from("<I", data)[0] != CAPTURE_MAGIC:
        raise ValueError(f"{path} is not a little-endian sysprof capture")
    offset = CAPTURE_HEADER_SIZE
    while offset + CAPTURE_FRAME.size <= len(data):
        length, _cpu, _pid, start, kind, _padding = CAPTURE_FRAME.unpack_from(data, offset)
        if length < CAPTURE_FRAME.size:
            break
        if kind & 0xFF == CAPTURE_MARK_KIND:
            body = offset + CAPTURE_FRAME.size
            duration, group, name = CAPTURE_MARK.unpack_from(data, body)
            message = data[body + CAPTURE_MARK.size : offset + length]
            yield (
                start,
                duration,
                _text(group),
                _text(name),
                _text(message),
            )
        offset += length


def _text(raw: bytes) -> str:
    return raw.split(b"\0", 1)[0].decode(errors="replace")


def percentile(values: list[int], fraction: float) -> int:
    if not values:
        return 0
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, round(fraction * (len(ordered) - 1)))]


PHASE_MARKS = {
    "Frameclock layout": "layout",
    "Validate CSS": "css",
    "Widget snapshot": "snapshot",
    "Widget render": "render",
    "Event": "events",
}


def frame_summary(capture: Path, start_ns: int, end_ns: int) -> dict[str, float]:
    cycles: list[int] = []
    totals = dict.fromkeys(PHASE_MARKS.values(), 0)
    for begin, duration, group, name, _message in read_marks(capture):
        if group != "GTK" or not start_ns <= begin <= end_ns:
            continue
        if name == "Frameclock cycle":
            cycles.append(duration)
        elif name in PHASE_MARKS:
            totals[PHASE_MARKS[name]] += duration
    seconds = (end_ns - start_ns) / 1e9
    summary = {
        "frames": len(cycles),
        "fps": len(cycles) / seconds if seconds > 0 else 0.0,
        "frame_p50_ms": percentile(cycles, 0.50) / 1e6,
        "frame_p95_ms": percentile(cycles, 0.95) / 1e6,
        "frame_max_ms": max(cycles, default=0) / 1e6,
        "frames_over_budget": sum(1 for cycle in cycles if cycle > FRAME_BUDGET_NS),
    }
    summary.update({f"{phase}_ms": total / 1e6 for phase, total in totals.items()})
    return summary


def _cpu_ms(stat: Path) -> float:
    fields = stat.read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) * 1000 / CLOCK_TICKS


def main_thread_cpu_ms(pid: int) -> float:
    return _cpu_ms(Path(f"/proc/{pid}/task/{pid}/stat"))


def process_cpu_ms(pid: int) -> float:
    return _cpu_ms(Path(f"/proc/{pid}/stat"))


def main_thread_wakeups(pid: int) -> int:
    total = 0
    for line in Path(f"/proc/{pid}/task/{pid}/status").read_text().splitlines():
        if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
            total += int(line.split()[1])
    return total


def find_child(parent: int, prefix: str, timeout: float = 20.0) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for task in Path(f"/proc/{parent}/task").iterdir():
            for child in (task / "children").read_text().split():
                try:
                    if Path(f"/proc/{child}/comm").read_text().startswith(prefix):
                        return int(child)
                except OSError:
                    continue
        time.sleep(0.05)
    raise TimeoutError(f"no {prefix} child of {parent}")


def wait_until_quiet(pid: int, *, minimum: float, timeout: float = 60.0) -> None:
    time.sleep(minimum)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        before = main_thread_cpu_ms(pid)
        time.sleep(1.0)
        if main_thread_cpu_ms(pid) - before < 30:
            return
    raise TimeoutError("strata did not settle")


@dataclass
class Driver:
    connection: XTestConnection

    def key(self, keysym: int, *, control: bool = False) -> None:
        held = [CONTROL] if control else []
        if self.connection.key_needs_shift(keysym):
            held.append(SHIFT)
        for modifier in held:
            self.connection.key(modifier, True)
        self.connection.key(keysym, True)
        self.connection.key(keysym, False)
        for modifier in reversed(held):
            self.connection.key(modifier, False)
        time.sleep(0.05)

    def type_text(self, text: str) -> None:
        for character in text:
            self.key(ord(character))

    def scroll(self, input_kind: str, steps: int, rate: float) -> None:
        if input_kind == "keys":
            forward, back = (lambda: self._tap(DOWN)), (lambda: self._tap(UP))
        else:
            forward, back = (lambda: self._click(WHEEL_DOWN)), (lambda: self._click(WHEEL_UP))
        for action in (forward, back):
            next_at = time.monotonic()
            for _ in range(steps):
                action()
                next_at += 1.0 / rate
                delay = next_at - time.monotonic()
                if delay > 0:
                    time.sleep(delay)

    def _click(self, button: int) -> None:
        self.connection.button(button, True)
        self.connection.button(button, False)

    def _tap(self, keysym: int) -> None:
        self.connection.key(keysym, True)
        self.connection.key(keysym, False)


@dataclass
class RunSpec:
    label: str
    binary: Path
    mode: str
    fixture: str
    round: int

    def name(self) -> str:
        return f"{self.label}-{self.mode}-{self.fixture}-{self.round}"


@dataclass
class Settings:
    fixtures: Path
    output: Path
    input_kind: str
    steps: int
    rate: float
    idle_seconds: float
    cpu_quota: int | None
    animations: bool
    perf_phase: str | None
    screenshots: bool
    keep_captures: bool
    preferences: dict[str, object] = field(default_factory=dict)


def application_command(
    binary: Path,
    folder: Path,
    capture: Path,
    cpu_quota: int | None,
    environment: dict[str, str],
) -> list[str]:
    command = [str(binary), str(folder)]
    if cpu_quota is not None:
        uid = os.getuid()
        command = [
            "env",
            f"DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus",
            f"XDG_RUNTIME_DIR=/run/user/{uid}",
            "systemd-run", "--user", "--scope", "--quiet",
            "-p", f"CPUQuota={cpu_quota}%", "-p", "CPUQuotaPeriodSec=10ms",
            "--", "env", *(f"{key}={value}" for key, value in environment.items()),
            *command,
        ]
    return [
        "sysprof-cli", "--force", "--gtk", "--no-perf", "--no-cpu", "--no-memory",
        "--no-disk", "--no-network", "--no-battery", "--no-sysprofd", "--no-decode",
        str(capture), "--", *command,
    ]


def start_perf(pid: int, profile: Path) -> subprocess.Popen:
    profile.parent.mkdir(parents=True, exist_ok=True)
    recorder = subprocess.Popen(
        ["perf", "record", "-q", "-F", "1999", "--call-graph", "fp", "-p", str(pid),
         "-o", str(profile)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(0.5)
    return recorder


def stop_perf(recorder: subprocess.Popen | None) -> None:
    if recorder is not None:
        recorder.send_signal(signal.SIGINT)
        recorder.wait(timeout=60)


def run_once(spec: RunSpec, settings: Settings, display: HeadlessDisplay) -> dict:
    home = TestEnvironment()
    home.write_preferences(
        {"browser_mode": spec.mode, "reduce_motion": False, **settings.preferences}
    )
    animations = "true" if settings.animations else "false"
    (home.config_home / "gtk-4.0" / "settings.ini").write_text(
        f"[Settings]\ngtk-enable-animations={animations}\ngtk-theme-name=Adwaita\n"
        "gtk-icon-theme-name=Adwaita\ngtk-cursor-blink=false\n"
    )
    environment = process_environment()
    environment.update(home.variables())
    environment.update(display.environment)
    environment.update({"GTK_A11Y": "none", "NO_AT_BRIDGE": "1", "RUST_LOG": "info"})
    environment.pop("GTK_OVERLAY_SCROLLING", None)
    capture = settings.output / "captures" / f"{spec.name()}.syscap"
    capture.parent.mkdir(parents=True, exist_ok=True)
    start = settings.fixtures / ("clip-source" if spec.fixture == "clipboard" else spec.fixture)
    profiler = ManagedProcess.spawn(
        "sysprof",
        application_command(spec.binary, start, capture, settings.cpu_quota, environment),
        log_dir=home.root,
        env=environment,
    )
    connection = None
    try:
        pid = find_child(profiler.popen.pid, "strata")
        wait_until_quiet(pid, minimum=4.0)
        connection = XTestConnection(display.display)
        driver = Driver(connection)
        if spec.fixture == "clipboard":
            connection.motion(*POINTER[spec.mode])
            driver.key(ord("a"), control=True)
            driver.key(ord("c"), control=True)
            driver.key(ord("l"), control=True)
            driver.type_text(str(settings.fixtures / "mixed"))
            driver.key(RETURN)
            wait_until_quiet(pid, minimum=2.0)
        connection.motion(*POINTER[spec.mode])
        time.sleep(0.5)
        if settings.screenshots:
            shot = settings.output / "screens" / f"{spec.name()}.png"
            shot.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(
                ["import", "-silent", "-window", "root", str(shot)],
                env={**environment, "DISPLAY": display.display},
                check=False,
            )
        driver.scroll(settings.input_kind, settings.steps, settings.rate)
        wait_until_quiet(pid, minimum=1.0)

        recorder = None
        if settings.perf_phase == "scroll":
            recorder = start_perf(pid, settings.output / "perf" / f"{spec.name()}-scroll.data")
        main_before = main_thread_cpu_ms(pid)
        process_before = process_cpu_ms(pid)
        scroll_start = time.monotonic_ns()
        driver.scroll(settings.input_kind, settings.steps, settings.rate)
        scroll_end = time.monotonic_ns()
        stop_perf(recorder)
        scroll_main = main_thread_cpu_ms(pid) - main_before
        scroll_process = process_cpu_ms(pid) - process_before

        wait_until_quiet(pid, minimum=1.0)
        recorder = None
        if settings.perf_phase == "idle":
            recorder = start_perf(pid, settings.output / "perf" / f"{spec.name()}-idle.data")
        idle_main_before = main_thread_cpu_ms(pid)
        wakeups_before = main_thread_wakeups(pid)
        idle_start = time.monotonic_ns()
        time.sleep(settings.idle_seconds)
        idle_end = time.monotonic_ns()
        idle_main = main_thread_cpu_ms(pid) - idle_main_before
        idle_wakeups = main_thread_wakeups(pid) - wakeups_before
        stop_perf(recorder)
    finally:
        if connection is not None:
            connection.close()
        try:
            os.killpg(profiler.popen.pid, signal.SIGINT)
            profiler.popen.wait(timeout=30)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            terminate(profiler.popen)
        home.cleanup()

    scroll_seconds = (scroll_end - scroll_start) / 1e9
    idle_seconds = (idle_end - idle_start) / 1e9
    steps = 2 * settings.steps
    result = {
        "label": spec.label,
        "mode": spec.mode,
        "fixture": spec.fixture,
        "round": spec.round,
        "input": settings.input_kind,
        "cpu_quota": settings.cpu_quota,
        "scroll_seconds": scroll_seconds,
        "main_cpu_ms": scroll_main,
        "main_cpu_per_step_ms": scroll_main / steps,
        "main_cpu_percent": scroll_main / 10 / scroll_seconds,
        "process_cpu_ms": scroll_process,
        "idle_main_cpu_percent": idle_main / 10 / idle_seconds,
        "idle_wakeups_per_second": idle_wakeups / idle_seconds,
    }
    result.update(frame_summary(capture, scroll_start, scroll_end))
    result["idle_fps"] = frame_summary(capture, idle_start, idle_end)["fps"]
    if not settings.keep_captures:
        capture.unlink(missing_ok=True)
    return result


REPORTED = (
    ("main_cpu_per_step_ms", "median CPU/step ms"),
    ("main_cpu_percent", "main CPU %"),
    ("frame_p50_ms", "frame p50 ms"),
    ("frame_p95_ms", "frame p95 ms"),
    ("frame_max_ms", "frame max ms"),
    ("frames_over_budget", "frames >16.7 ms"),
    ("css_ms", "CSS ms"),
    ("layout_ms", "layout ms"),
    ("snapshot_ms", "snapshot ms"),
    ("render_ms", "render ms"),
    ("idle_wakeups_per_second", "idle wakeups/s"),
    ("idle_fps", "idle frames/s"),
)


def summarize(results: list[dict]) -> str:
    groups: dict[tuple[str, str, str], list[dict]] = {}
    for result in results:
        groups.setdefault((result["mode"], result["fixture"], result["label"]), []).append(result)
    lines = [
        "| mode | fixture | build | runs | fastest CPU/step ms | "
        + " | ".join(title for _, title in REPORTED)
        + " |",
        "|---|---|---|---:|---:|" + "---:|" * len(REPORTED),
    ]
    for (mode, fixture, label), runs in sorted(groups.items()):
        cells = [f"{min(run['main_cpu_per_step_ms'] for run in runs):.2f}"]
        for key, _ in REPORTED:
            value = statistics.median(run[key] for run in runs)
            cells.append(f"{value:.0f}" if key == "frames_over_budget" else f"{value:.2f}")
        lines.append(f"| {mode} | {fixture} | {label} | {len(runs)} | " + " | ".join(cells) + " |")
    return "\n".join(lines)


def parse_binary(value: str) -> tuple[str, Path]:
    label, separator, path = value.partition("=")
    if not separator:
        label, path = Path(value).name, value
    binary = Path(path).resolve()
    if not binary.is_file():
        raise argparse.ArgumentTypeError(f"{binary} is not a file")
    return label, binary


def parse_preference(value: str) -> tuple[str, object]:
    key, separator, raw = value.partition("=")
    if not separator or not key:
        raise argparse.ArgumentTypeError(f"{value!r} is not KEY=VALUE")
    if raw in ("true", "false"):
        return key, raw == "true"
    try:
        return key, int(raw)
    except ValueError:
        return key, raw


def parse_list(value: str, allowed: tuple[str, ...]) -> list[str]:
    chosen = [item for item in value.split(",") if item]
    unknown = sorted(set(chosen) - set(allowed))
    if unknown:
        raise argparse.ArgumentTypeError(f"unknown {unknown}; choose from {', '.join(allowed)}")
    return chosen


def missing_tools(arguments: argparse.Namespace) -> list[str]:
    needed = dict(TOOLS)
    if arguments.perf:
        needed["perf"] = "perf"
    if arguments.screenshots:
        needed["import"] = "ImageMagick"
    if arguments.cpu_quota is not None:
        needed["systemd-run"] = "systemd"
    return [package for tool, package in needed.items() if shutil.which(tool) is None]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", action="append", type=parse_binary, required=True,
                        help="LABEL=PATH of a strata binary; repeat to compare builds")
    parser.add_argument("--modes", type=lambda value: parse_list(value, MODES),
                        default=list(MODES), help="comma-separated: columns,list,icons")
    parser.add_argument("--fixtures", type=lambda value: parse_list(value, FIXTURES),
                        default=["mixed", "images", "folders"],
                        help="comma-separated: mixed,images,folders,clipboard")
    parser.add_argument("--input", choices=INPUTS, default="wheel",
                        help="turn the mouse wheel, or press Down and then Up")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--steps", type=int, default=100,
                        help="wheel notches or key presses in each direction")
    parser.add_argument("--rate", type=float, default=30.0, help="steps per second")
    parser.add_argument("--idle-seconds", type=float, default=5.0)
    parser.add_argument("--cpu-quota", type=int, metavar="PERCENT",
                        help="limit Strata to this share of one CPU")
    parser.add_argument("--no-animations", action="store_true",
                        help="turn GTK animations and CSS transitions off")
    parser.add_argument("--preference", action="append", type=parse_preference, default=[],
                        metavar="KEY=VALUE", help="override a saved preference")
    parser.add_argument("--perf", choices=("scroll", "idle"), metavar="PHASE",
                        help="record a perf call graph of the measured scroll or the idle window")
    parser.add_argument("--screenshots", action="store_true",
                        help="save what each configuration shows before scrolling")
    parser.add_argument("--keep-captures", action="store_true",
                        help="keep the sysprof capture of each run")
    parser.add_argument("--fixture-root", type=Path,
                        default=REPOSITORY / "target" / "scroll-bench" / "fixtures")
    parser.add_argument("--output", type=Path,
                        default=REPOSITORY / "target" / "scroll-bench" / "results")
    arguments = parser.parse_args()

    missing = missing_tools(arguments)
    if missing:
        parser.error(f"install {', '.join(missing)}")
    ensure_fixtures(arguments.fixture_root)
    arguments.output.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    results_path = arguments.output / f"scroll-{stamp}.jsonl"
    settings = Settings(
        fixtures=arguments.fixture_root,
        output=arguments.output,
        input_kind=arguments.input,
        steps=arguments.steps,
        rate=arguments.rate,
        idle_seconds=arguments.idle_seconds,
        cpu_quota=arguments.cpu_quota,
        animations=not arguments.no_animations,
        perf_phase=arguments.perf,
        screenshots=arguments.screenshots,
        keep_captures=arguments.keep_captures,
        preferences=dict(arguments.preference),
    )
    display = HeadlessDisplay(width=WINDOW_WIDTH, height=WINDOW_HEIGHT)
    display.start()
    results = []
    try:
        for round_index in range(arguments.rounds):
            for mode in arguments.modes:
                for fixture in arguments.fixtures:
                    for label, binary in arguments.binary:
                        result = run_once(
                            RunSpec(label, binary, mode, fixture, round_index), settings, display
                        )
                        results.append(result)
                        with results_path.open("a") as handle:
                            handle.write(json.dumps(result) + "\n")
                        print(
                            f"{label:>10} {mode:>7} {fixture:>9} r{round_index}: "
                            f"{result['main_cpu_per_step_ms']:.2f} ms CPU/step, "
                            f"frame p95 {result['frame_p95_ms']:.2f} ms, "
                            f"{result['frames_over_budget']} over budget, "
                            f"idle {result['idle_wakeups_per_second']:.0f} wakeups/s",
                            flush=True,
                        )
    finally:
        display.stop()
    summary = summarize(results)
    (arguments.output / f"scroll-{stamp}.md").write_text(summary + "\n")
    print()
    print(summary)
    return 0


if __name__ == "__main__":
    sys.exit(main())
