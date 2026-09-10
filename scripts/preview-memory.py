#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Record opt-in preview traces alongside external NVIDIA and /proc samples."""

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
import xml.etree.ElementTree as ET

REPOSITORY = Path(__file__).resolve().parents[1]


def memory_mib(text):
    if not text or text.strip() in ("N/A", "[N/A]", "Not Supported"):
        return None
    value, unit = text.split()
    if unit != "MiB":
        raise ValueError(f"Unexpected NVIDIA memory unit: {unit}")
    return int(value)


def parse_nvidia(xml):
    root = ET.fromstring(xml)
    gpus = []
    processes = []
    for index, gpu in enumerate(root.findall("gpu")):
        gpus.append({
            "gpu": index,
            "name": gpu.findtext("product_name"),
            "total_mib": memory_mib(gpu.findtext("fb_memory_usage/total")),
            "used_mib": memory_mib(gpu.findtext("fb_memory_usage/used")),
        })
        for process in gpu.findall("processes/process_info"):
            processes.append({
                "gpu": index,
                "pid": int(process.findtext("pid")),
                "type": process.findtext("type"),
                "used_mib": memory_mib(process.findtext("used_memory")),
            })
    if not gpus:
        raise ValueError("NVIDIA reported no GPUs")
    return {"driver": root.findtext("driver_version"), "gpus": gpus, "processes": processes}


def query_nvidia():
    # Unlike --query-compute-apps, XML includes graphics-only clients.
    result = subprocess.run(
        ["nvidia-smi", "-q", "-x"], capture_output=True, text=True, timeout=5, check=True
    )
    return parse_nvidia(result.stdout)


def read_process(pid, proc=Path("/proc")):
    directory = proc / str(pid)
    try:
        status = dict(line.split(":", 1) for line in (directory / "status").read_text(errors="replace").splitlines())
        stat = (directory / "stat").read_text(errors="replace").rsplit(")", 1)[1].split()
        return {
            "pid": pid,
            "ppid": int(status["PPid"]),
            "start_ticks": int(stat[19]),
            "name": status["Name"].strip(),
            "rss_kib": int(status["VmRSS"].split()[0]) if "VmRSS" in status else None,
            "threads": int(status["Threads"]),
            "namespace_pids": [int(value) for value in status.get("NSpid", "").split()],
        }
    except (OSError, ValueError, KeyError, IndexError):
        return None


def process_snapshot():
    result = {}
    for directory in Path("/proc").iterdir():
        if directory.name.isdecimal():
            pid = int(directory.name)
            process = read_process(pid)
            if process:
                result[pid] = process
    return result


def select_descendants(snapshot, tracked):
    # Retain observed descendants across reparenting, but not PID reuse.
    selected = {
        pid: process for pid, process in snapshot.items()
        if tracked.get(pid) == process["start_ticks"]
    }
    while True:
        added = {
            pid: process for pid, process in snapshot.items()
            if pid not in selected and process["ppid"] in selected
        }
        if not added:
            break
        selected.update(added)
    tracked.update({pid: process["start_ticks"] for pid, process in selected.items()})
    return selected


def fd_category(target):
    if target.startswith("socket:["):
        return "socket"
    if target.startswith("pipe:["):
        return "pipe"
    if target.startswith(("memfd:", "/memfd:")):
        return "memfd"
    if target.startswith("anon_inode:"):
        kind = target.removeprefix("anon_inode:").strip("[]")
        return "anon_inode:" + (kind if kind in {
            "eventfd", "eventpoll", "timerfd", "signalfd", "inotify", "pidfd", "io_uring", "dmabuf", "sync_file"
        } else "other")
    if re.fullmatch(r"/dev/nvidia\d+", target):
        return "gpu:nvidia-device"
    if target in ("/dev/nvidiactl", "/dev/nvidia-uvm", "/dev/nvidia-uvm-tools"):
        return "gpu:" + target.removeprefix("/dev/")
    if re.fullmatch(r"/dev/dri/renderD\d+", target):
        return "gpu:drm-render"
    if re.fullmatch(r"/dev/dri/card\d+", target):
        return "gpu:drm-card"
    if target.startswith("/dev/"):
        return "device:other"
    return "filesystem:deleted" if target.endswith(" (deleted)") else "filesystem:other"


def thread_category(name):
    # Never emit arbitrary comm strings: applications can put filenames in them.
    if name in {"strata", "gmain", "gdbus", "dconf worker", "gstglcontext", "gstglthread"}:
        return name
    if name.startswith("pool-"):
        return "glib-pool"
    if re.fullmatch(r"(?:multi)?queue\d+:(?:src|sink)(?:_\d+)?", name):
        return "gst-queue"
    if name.lower().startswith(("cuda-", "cuda_", "nvidia")):
        return "nvidia-worker"
    return "other"


def resource_snapshot(process, proc=Path("/proc")):
    directory = proc / str(process["pid"])
    result = {}
    for kind, child_directory in (("fd", directory / "fd"), ("thread", directory / "task")):
        try:
            children = list(child_directory.iterdir())
        except OSError:
            result[kind + "_error"] = "unavailable"
            continue
        categories = Counter()
        unreadable = 0
        for child in children:
            try:
                category = fd_category(os.readlink(child)) if kind == "fd" else thread_category(
                    (child / "comm").read_text(errors="replace").strip()
                )
                categories[category] += 1
            except OSError:
                unreadable += 1
        result[kind + "_listed"] = len(children)
        result[kind + "_unreadable"] = unreadable
        result[kind + "_categories"] = dict(categories)
    try:
        fields = {}
        for line in (directory / "smaps_rollup").read_text().splitlines():
            key, separator, value = line.partition(":")
            if separator and key in {"Rss", "Pss", "Private_Clean", "Private_Dirty", "Shared_Clean", "Shared_Dirty", "Swap", "Anonymous"}:
                number, unit = value.split()
                if unit != "kB":
                    raise ValueError("Unexpected smaps unit")
                fields[key + "_kib"] = int(number)
        if "Pss_kib" not in fields:
            raise ValueError("Missing PSS")
        result["memory"] = fields
    except (OSError, ValueError):
        result["memory_error"] = "unavailable"
    current = read_process(process["pid"], proc)
    if current is None or current["start_ticks"] != process["start_ticks"]:
        return {"error": "process_exited_or_changed"}
    return result


def sample(tracked, *, resources=False):
    started = time.time_ns() // 1_000_000
    processes = select_descendants(process_snapshot(), tracked)
    for pid, process in processes.items():
        try:
            process["fds"] = sum(1 for _ in (Path("/proc") / str(pid) / "fd").iterdir())
        except OSError:
            process["fds"] = None
        if resources:
            process["resources"] = resource_snapshot(process)
    record = {"unix_ms": started, "processes": list(processes.values())}
    try:
        nvidia = query_nvidia()
        record["gpus"] = nvidia["gpus"]
        record["gpu_processes"] = [p for p in nvidia["processes"] if p["pid"] in processes]
    except (OSError, subprocess.SubprocessError, ValueError, ET.ParseError) as error:
        record["gpu_error"] = str(error)
    record["sample_end_unix_ms"] = time.time_ns() // 1_000_000
    return record


def debug_environment(profile):
    profile.mkdir(parents=True, exist_ok=True, mode=0o700)
    config = profile / "config"
    if not config.exists():
        config.mkdir(mode=0o700)
        original = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "strata"
        (config / "strata").mkdir(mode=0o700)
        settings = original / "settings.toml"
        if settings.is_file():
            shutil.copyfile(settings, config / "strata/settings.toml")
        if (original / "themes").is_dir():
            shutil.copytree(original / "themes", config / "strata/themes")
    environment = dict(os.environ)
    for key in ("DBUS_SESSION_BUS_ADDRESS", "AT_SPI_BUS_ADDRESS", "STRATA_PREVIEW_TRACE_JOB"):
        environment.pop(key, None)
    environment.update({"STRATA_PREVIEW_TRACE": "1", "GTK_A11Y": "none", "NO_AT_BRIDGE": "1"})
    for kind in ("CONFIG", "CACHE", "DATA", "STATE"):
        directory = profile / kind.lower()
        directory.mkdir(exist_ok=True, mode=0o700)
        environment[f"XDG_{kind}_HOME"] = str(directory)
    return environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--binary", type=Path, help="Launch this binary (default: target/debug/strata)")
    source.add_argument("--pid", type=int, help="Only sample an existing PID; cannot enable its traces")
    parser.add_argument("--output", type=Path, help="New capture directory (must not already exist)")
    parser.add_argument("--profile", type=Path, default=REPOSITORY / "target/preview-memory-profile",
                        help="Persistent private settings/cache, seeded from current Strata settings once")
    parser.add_argument("--interval", type=float, default=0.5, help="Sampling interval in seconds")
    parser.add_argument("--seconds", type=float, help="Stop sampling after this duration; leave app running")
    parser.add_argument("--resources", action="store_true",
                        help="Also sample path-free FD/thread categories and PSS every five seconds")
    args = parser.parse_args()
    if not 0.1 <= args.interval <= 60 or (args.seconds is not None and not 0 < args.seconds < float("inf")):
        parser.error("Use interval 0.1–60 seconds and a positive finite duration")
    if args.pid is not None and args.pid <= 1:
        parser.error("PID must be greater than 1")
    nvidia = query_nvidia()
    binary = (args.binary or REPOSITORY / "target/debug/strata").resolve()
    if args.pid is None:
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"Executable not found: {binary}; build with cargo build --locked")
        if not shutil.which("dbus-run-session"):
            parser.error("dbus-run-session is required for a separate debug instance")
    else:
        if read_process(args.pid) is None:
            parser.error("PID is unavailable")
    output = (args.output or REPOSITORY / "target/preview-memory" / time.strftime("%Y%m%d-%H%M%S")).resolve()
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    app = None
    app_log = None
    metadata = {
        "started_unix_ms": time.time_ns() // 1_000_000,
        "driver": nvidia["driver"], "gpus": nvidia["gpus"], "interval_seconds": args.interval,
        "resource_interval_seconds": 5 if args.resources else None,
        "environment": {key: os.environ.get(key) for key in ("GSK_RENDERER", "GDK_BACKEND", "GTK_MEDIA")},
        "notes": "Missing GPU process records and N/A are not zero. GPU totals include unrelated applications.",
    }
    try:
        if args.pid is None:
            with binary.open("rb") as executable:
                metadata["binary_sha256"] = hashlib.file_digest(executable, "sha256").hexdigest()
            app_log = (output / "app.log").open("x")
            app = subprocess.Popen(
                ["dbus-run-session", "--", str(binary)],
                env=debug_environment(args.profile.resolve()),
                stdout=app_log, stderr=subprocess.STDOUT, start_new_session=True,
            )
            root_pid = app.pid
            metadata["profile"] = str(args.profile.resolve())
        else:
            root_pid = args.pid
        root = read_process(root_pid)
        if root is None:
            raise RuntimeError("Process exited before capture started; inspect app.log")
        tracked = {root_pid: root["start_ticks"]}
        metadata["root_pid"] = root_pid
        (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
        print(f"Capture: {output}", flush=True)
        print("Preview files, then CLOSE THE DEBUG WINDOW to finish (5s tail).", flush=True)
        print("Ctrl+C stops sampling without killing the app. Settings changes affect only the debug profile.", flush=True)
        started = time.monotonic()
        exited_at = None
        last_report = 0.0
        next_resources = started
        with (output / "samples.jsonl").open("x", buffering=1) as stream:
            while True:
                cycle = time.monotonic()
                include_resources = args.resources and cycle >= next_resources
                record = sample(tracked, resources=include_resources)
                if include_resources:
                    next_resources = time.monotonic() + 5
                stream.write(json.dumps(record) + "\n")
                if cycle - last_report >= 5:
                    gpu_values = [p["used_mib"] for p in record.get("gpu_processes", [])]
                    known = [value for value in gpu_values if value is not None]
                    gpu = f"{sum(known)} MiB reported" if known else "unavailable/no reported context"
                    print(f"{cycle - started:6.1f}s  GPU tree: {gpu}; processes: {len(record['processes'])}", flush=True)
                    last_report = cycle
                alive = any(p["pid"] == root_pid and p["start_ticks"] == root["start_ticks"]
                            for p in record["processes"])
                if app is not None and app.poll() is not None:
                    alive = False
                if not alive and exited_at is None:
                    exited_at = cycle
                if (exited_at is not None and cycle - exited_at >= 5
                        or args.seconds is not None and cycle - started >= args.seconds):
                    break
                time.sleep(max(0, args.interval - (time.monotonic() - cycle)))
    except KeyboardInterrupt:
        print("\nSampling stopped; the debug app, if still open, was left running.")
    finally:
        if app_log is not None:
            app_log.close()
        (output / "finished.json").write_text(json.dumps({
            "unix_ms": time.time_ns() // 1_000_000,
            "application_exit_code": app.poll() if app is not None else None,
        }) + "\n")
        print(f"Saved: {output}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
