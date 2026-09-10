# SPDX-License-Identifier: MIT

import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("preview_memory", Path(__file__).with_name("preview-memory.py"))
memory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(memory)

XML = """<nvidia_smi_log><driver_version>test</driver_version><gpu>
<product_name>Test GPU</product_name>
<fb_memory_usage><total>8192 MiB</total><used>1000 MiB</used></fb_memory_usage>
<processes>
<process_info><pid>10</pid><type>G</type><used_memory>200 MiB</used_memory></process_info>
<process_info><pid>11</pid><type>C+G</type><used_memory>N/A</used_memory></process_info>
<process_info><pid>99</pid><type>C</type><used_memory>800 MiB</used_memory></process_info>
</processes></gpu></nvidia_smi_log>"""


def process(pid, ppid, start=1):
    return {"pid": pid, "ppid": ppid, "start_ticks": start}


class PreviewMemoryTests(unittest.TestCase):
    def test_graphics_clients_are_included_and_unavailable_is_not_zero(self):
        result = memory.parse_nvidia(XML)
        self.assertEqual(result["processes"][0], {"gpu": 0, "pid": 10, "type": "G", "used_mib": 200})
        self.assertIsNone(result["processes"][1]["used_mib"])
        self.assertEqual(result["gpus"][0]["used_mib"], 1000)
        with self.assertRaises(ValueError):
            memory.parse_nvidia("<nvidia_smi_log/>")

    def test_descendants_include_helpers_and_survive_reparenting(self):
        tracked = {10: 1}
        snapshot = {p["pid"]: p for p in [process(10, 1), process(11, 10), process(12, 11), process(99, 1)]}
        self.assertEqual(set(memory.select_descendants(snapshot, tracked)), {10, 11, 12})
        self.assertEqual(set(memory.select_descendants({12: process(12, 1)}, tracked)), {12})

    def test_pid_reuse_does_not_adopt_unrelated_processes(self):
        tracked = {10: 1, 11: 1}
        reused = {10: process(10, 1, 2), 11: process(11, 10, 2), 12: process(12, 11)}
        self.assertEqual(memory.select_descendants(reused, tracked), {})

    def test_samples_exclude_unrelated_gpu_clients(self):
        with patch.object(memory, "process_snapshot", return_value={10: process(10, 1)}), \
                patch.object(memory, "query_nvidia", return_value=memory.parse_nvidia(XML)):
            result = memory.sample({10: 1})
        self.assertEqual([p["pid"] for p in result["gpu_processes"]], [10])
        self.assertGreaterEqual(result["sample_end_unix_ms"], result["unix_ms"])
        json.dumps(result)

    def test_resource_collection_is_opt_in_and_attached_to_each_process(self):
        with patch.object(memory, "process_snapshot", side_effect=lambda: {10: process(10, 1)}), \
                patch.object(memory, "query_nvidia", return_value=memory.parse_nvidia(XML)), \
                patch.object(memory, "resource_snapshot", return_value={"memory": {"Pss_kib": 512}}) as snapshot:
            plain = memory.sample({10: 1})
            snapshot.assert_not_called()
            self.assertNotIn("resources", plain["processes"][0])
            detailed = memory.sample({10: 1}, resources=True)
            snapshot.assert_called_once()
            self.assertEqual(detailed["processes"][0]["resources"]["memory"]["Pss_kib"], 512)

    def test_query_failure_is_reported_not_zero(self):
        with patch.object(memory, "process_snapshot", return_value={}), \
                patch.object(memory, "query_nvidia", side_effect=ValueError("unsupported")):
            result = memory.sample({10: 1})
        self.assertEqual(result["gpu_error"], "unsupported")
        self.assertNotIn("gpu_processes", result)

    def test_proc_identity_and_namespace_mapping(self):
        with tempfile.TemporaryDirectory() as temporary:
            proc = Path(temporary)
            directory = proc / "123"
            directory.mkdir()
            (directory / "status").write_text("Name:\tstrata\nPPid:\t12\nVmRSS:\t2048 kB\nThreads:\t4\nNSpid:\t123\t2\n")
            fields = ["S"] + ["0"] * 18 + ["7654"]
            (directory / "stat").write_text("123 (name with ) spaces) " + " ".join(fields))
            result = memory.read_process(123, proc)
            self.assertEqual(result["start_ticks"], 7654)
            self.assertEqual(result["namespace_pids"], [123, 2])
            self.assertEqual(result["rss_kib"], 2048)
            self.assertIsNone(memory.read_process(999, proc))

    def test_fd_categories_do_not_include_paths_or_arbitrary_inode_names(self):
        cases = {
            "socket:[123]": "socket", "pipe:[99]": "pipe",
            "anon_inode:[eventfd]": "anon_inode:eventfd",
            "anon_inode:[private-document]": "anon_inode:other",
            "/dev/nvidia0": "gpu:nvidia-device", "/dev/nvidiactl": "gpu:nvidiactl",
            "/dev/dri/renderD128": "gpu:drm-render", "/dev/dri/card1": "gpu:drm-card",
            "/memfd:private-document (deleted)": "memfd",
            "/home/private/document.mp4": "filesystem:other",
            "/home/private/document.mp4 (deleted)": "filesystem:deleted",
        }
        for target, expected in cases.items():
            with self.subTest(target=target):
                self.assertEqual(memory.fd_category(target), expected)

    def test_thread_categories_do_not_emit_arbitrary_comm_names(self):
        for name, expected in {
            "gstglcontext": "gstglcontext", "gdbus": "gdbus",
            "pool-strata": "glib-pool", "multiqueue12:src_3": "gst-queue",
            "cuda-EvtHandlr": "nvidia-worker", "private-file.txt": "other",
        }.items():
            with self.subTest(name=name):
                self.assertEqual(memory.thread_category(name), expected)

    def test_resource_snapshot_reports_categories_pss_and_racing_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            proc = Path(temporary)
            directory = proc / "123"
            (directory / "fd").mkdir(parents=True)
            (directory / "fd/0").symlink_to("socket:[42]")
            (directory / "fd/1").symlink_to("/home/private/document.mp4")
            (directory / "fd/2").touch()
            for tid, name in ((123, "strata"), (124, "gstglcontext"), (125, "private-file")):
                task = directory / "task" / str(tid)
                task.mkdir(parents=True)
                (task / "comm").write_text(name)
            (directory / "task/126").mkdir()
            (directory / "smaps_rollup").write_text(
                "0000-ffff ---p 0 00:00 0 /private/path\nRss: 1024 kB\nPss: 512 kB\nPrivate_Dirty: 100 kB\n"
            )
            identity = process(123, 1)
            with patch.object(memory, "read_process", return_value=identity):
                result = memory.resource_snapshot(identity, proc)
            self.assertEqual(result["fd_categories"], {"socket": 1, "filesystem:other": 1})
            self.assertEqual(result["fd_listed"], 3)
            self.assertEqual(result["fd_unreadable"], 1)
            self.assertEqual(result["thread_categories"], {"strata": 1, "gstglcontext": 1, "other": 1})
            self.assertEqual(result["thread_unreadable"], 1)
            self.assertEqual(result["memory"]["Pss_kib"], 512)
            self.assertNotIn("private", json.dumps(result))
            with patch.object(memory, "read_process", return_value=process(123, 1, 2)):
                self.assertEqual(memory.resource_snapshot(identity, proc), {"error": "process_exited_or_changed"})

    def test_unavailable_resources_are_explicit_not_empty_success(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(memory, "read_process", return_value=process(123, 1)):
            result = memory.resource_snapshot(process(123, 1), Path(temporary))
        self.assertEqual(result, {"fd_error": "unavailable", "thread_error": "unavailable", "memory_error": "unavailable"})

    def test_private_profile_is_seeded_once_and_does_not_modify_user_settings(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / "original/strata"
            original.mkdir(parents=True)
            source = original / "settings.toml"
            source.write_text("hardware_accelerated_video_previews = false\n")
            with patch.dict(os.environ, {"XDG_CONFIG_HOME": str(original.parent),
                                        "DBUS_SESSION_BUS_ADDRESS": "desktop", "AT_SPI_BUS_ADDRESS": "desktop"}):
                environment = memory.debug_environment(root / "profile")
                copied = Path(environment["XDG_CONFIG_HOME"]) / "strata/settings.toml"
                self.assertEqual(copied.read_text(), source.read_text())
                copied.write_text("hardware_accelerated_video_previews = true\n")
                memory.debug_environment(root / "profile")
                self.assertIn("true", copied.read_text())
                self.assertIn("false", source.read_text())
            self.assertNotIn("DBUS_SESSION_BUS_ADDRESS", environment)
            self.assertNotIn("AT_SPI_BUS_ADDRESS", environment)
            self.assertEqual(environment["STRATA_PREVIEW_TRACE"], "1")


if __name__ == "__main__":
    unittest.main()
