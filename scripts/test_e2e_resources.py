# SPDX-License-Identifier: MIT

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "e2e_resources", Path(__file__).resolve().parents[1] / "tests/e2e/harness/resources.py"
)
RESOURCES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RESOURCES)
GIB = RESOURCES.GIB


class WorkerCountTests(unittest.TestCase):
    def test_cpu_memory_and_safety_cap(self):
        for cpus, memory, expected in ((32, 38, 16), (4, 15, 2), (128, 256, 16),
                                       (32, 7, 3), (1, 1, 1), (0.5, 0, 1)):
            with self.subTest(cpus=cpus, memory=memory):
                self.assertEqual(RESOURCES.worker_count(cpus, memory * GIB), expected)

    def test_manual_override_and_serial(self):
        self.assertEqual(RESOURCES.worker_count(4, GIB, "8"), 8)
        self.assertEqual(RESOURCES.worker_count(32, 60 * GIB, "1"), 1)

    def test_invalid_overrides(self):
        for value in ("", "0", "-1", "1.5", "all", " auto", "²"):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "positive integer"):
                RESOURCES.worker_count(32, 60 * GIB, value)


class ResourceDetectionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.proc = self.root / "proc"
        (self.proc / "self").mkdir(parents=True)
        (self.proc / "meminfo").write_text(f"MemAvailable: {40 * GIB // 1024} kB\n")
        self.mount = self.root / "cgroup"
        self.mount.mkdir()
        affinity = patch.object(RESOURCES.os, "sched_getaffinity", return_value=set(range(32)))
        affinity.start()
        self.addCleanup(affinity.stop)

    def mount_cgroup(self, membership="/job/worker", root="/", version=2):
        controllers = "" if version == 2 else "cpu,memory"
        filesystem = "cgroup2 cgroup rw" if version == 2 else "cgroup cgroup rw,cpu,memory"
        (self.proc / "self/cgroup").write_text(f"0:{controllers}:{membership}\n")
        (self.proc / "self/mountinfo").write_text(
            f"1 0 0:1 {root} {self.mount} rw - {filesystem}\n"
        )

    def limits(self, path, **files):
        path.mkdir(parents=True, exist_ok=True)
        for name, contents in files.items():
            (path / name).write_text(str(contents))

    def test_v2_ancestor_quota_and_remaining_memory(self):
        self.mount_cgroup()
        self.limits(self.mount, **{"cpu.max": "800000 100000", "memory.max": 10 * GIB,
                                  "memory.current": 3 * GIB})
        self.limits(self.mount / "job", **{"cpu.max": "350000 100000"})
        self.limits(self.mount / "job/worker", **{"cpu.max": "max 100000", "memory.max": "max"})
        self.assertEqual(RESOURCES.available_resources(self.proc), (3.5, 7 * GIB))

    def test_v1_limits(self):
        self.mount_cgroup(membership="/", version=1)
        self.limits(self.mount, **{"cpu.cfs_quota_us": 200000, "cpu.cfs_period_us": 100000,
                                  "memory.limit_in_bytes": 8 * GIB, "memory.usage_in_bytes": 2 * GIB})
        self.assertEqual(RESOURCES.available_resources(self.proc), (2, 6 * GIB))

    def test_mount_root_and_namespace_relative_membership(self):
        for membership, root in (("/host/job/worker", "/host/job"), ("/", "/host/job"),
                                 ("/../../host/job", "/")):
            with self.subTest(membership=membership):
                self.mount_cgroup(membership, root)
                self.limits(self.mount, **{"cpu.max": "100000 100000"})
                self.assertEqual(RESOURCES.available_resources(self.proc)[0], 1)
                self.assertTrue(all(p.is_relative_to(self.mount) for p in RESOURCES.cgroup_directories(self.proc)))

    def test_affinity_and_host_memory_are_upper_bounds(self):
        self.mount_cgroup(membership="/")
        self.limits(self.mount, **{"cpu.max": "6400000 100000", "memory.max": 100 * GIB,
                                  "memory.current": GIB})
        self.assertEqual(RESOURCES.available_resources(self.proc), (32, 40 * GIB))

    def test_missing_or_invalid_limits_and_cpu_fallback(self):
        self.mount_cgroup(membership="/")
        self.limits(self.mount, **{"cpu.max": "garbage", "memory.max": "max"})
        with patch.object(RESOURCES.os, "sched_getaffinity", side_effect=OSError), \
             patch.object(RESOURCES.os, "cpu_count", return_value=None):
            self.assertEqual(RESOURCES.available_resources(self.proc), (1, 40 * GIB))
        (self.proc / "meminfo").unlink()
        self.assertEqual(RESOURCES.available_resources(self.proc)[1], 0)

    def test_memory_pressure_never_selects_zero_workers(self):
        self.mount_cgroup(membership="/")
        self.limits(self.mount, **{"memory.max": GIB, "memory.current": 2 * GIB})
        cpus, memory = RESOURCES.available_resources(self.proc)
        self.assertEqual(memory, 0)
        self.assertEqual(RESOURCES.worker_count(cpus, memory), 1)


if __name__ == "__main__":
    unittest.main()
