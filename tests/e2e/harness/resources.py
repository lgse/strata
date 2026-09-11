# SPDX-License-Identifier: MIT
"""Conservative GUI concurrency from resources available to this process."""

from __future__ import annotations

import os
from pathlib import Path

GIB = 1024**3
MAX_AUTO_WORKERS = 16


def _read(path: Path) -> str:
    try:
        return path.read_text().strip()
    except OSError:
        return ""


def cgroup_directories(proc: Path = Path("/proc")) -> list[Path]:
    """Resolve v1/v2 mounts, including namespace roots and ancestor limits."""
    memberships = {}
    for line in _read(proc / "self/cgroup").splitlines():
        _, controllers, location = line.split(":", 2)
        for controller in controllers.split(","):
            memberships[controller] = location
    directories = set()
    for line in _read(proc / "self/mountinfo").splitlines():
        before, after = line.split(" - ", 1)
        fields, filesystem = before.split(), after.split()
        if filesystem[0] not in ("cgroup", "cgroup2"):
            continue
        root, mount = Path(fields[3]), Path(fields[4])
        controllers = [""] if filesystem[0] == "cgroup2" else filesystem[2].split(",")
        for controller in controllers:
            if controller not in memberships:
                continue
            location = Path(memberships[controller])
            # A namespace can hide the membership's host-side prefix. Its mount
            # root still exposes the applicable limit; never escape that mount.
            if ".." in location.parts or not location.is_relative_to(root):
                current = mount
            else:
                current = mount / location.relative_to(root)
            while True:
                directories.add(current)
                if current == mount:
                    break
                current = current.parent
    return sorted(directories)


def available_resources(proc: Path = Path("/proc")) -> tuple[float, int]:
    try:
        cpus = float(len(os.sched_getaffinity(0)))
    except (AttributeError, OSError):
        cpus = float(os.cpu_count() or 1)
    memory = 0
    for line in _read(proc / "meminfo").splitlines():
        if line.startswith("MemAvailable:"):
            memory = int(line.split()[1]) * 1024
    for directory in cgroup_directories(proc):
        quota = _read(directory / "cpu.max").split()
        if not quota:
            quota = [_read(directory / "cpu.cfs_quota_us"),
                     _read(directory / "cpu.cfs_period_us")]
        try:
            limit, period = map(int, quota)
            if limit > 0 and period > 0:
                cpus = min(cpus, limit / period)
        except ValueError:
            pass
        for maximum, usage in (("memory.max", "memory.current"),
                               ("memory.limit_in_bytes", "memory.usage_in_bytes")):
            try:
                remaining = max(0, int(_read(directory / maximum)) - int(_read(directory / usage)))
                memory = min(memory, remaining)
            except ValueError:
                pass
    return cpus, memory


def worker_count(cpus: float, memory: int, override: str = "auto") -> int:
    if override != "auto":
        if not override.isdecimal() or int(override) < 1:
            raise ValueError("STRATA_E2E_WORKERS must be 'auto' or a positive integer")
        return int(override)
    # Leave memory for the controller and infrastructure. Each worker runs
    # Strata, Xvfb, and two buses; CPU oversubscription destabilizes GUI timing.
    return max(1, min(MAX_AUTO_WORKERS, int(cpus / 2), (memory - GIB) // (2 * GIB)))
