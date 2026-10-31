# SPDX-License-Identifier: MIT

import argparse
import importlib.util
import struct
import sys
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "benchmark_scroll", Path(__file__).with_name("benchmark-scroll.py")
)
BENCHMARK = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = BENCHMARK
SPEC.loader.exec_module(BENCHMARK)

MS = 1_000_000


def frame(kind: int, start: int, body: bytes = b"") -> bytes:
    length = BENCHMARK.CAPTURE_FRAME.size + len(body)
    padded = length + (-length % 8)
    header = BENCHMARK.CAPTURE_FRAME.pack(padded, -1, 4242, start, kind, 0)
    return header + body + b"\0" * (padded - length)


def mark(start: int, duration: int, group: str, name: str, message: str = "") -> bytes:
    body = BENCHMARK.CAPTURE_MARK.pack(duration, group.encode(), name.encode())
    return frame(BENCHMARK.CAPTURE_MARK_KIND, start, body + message.encode() + b"\0")


def capture(directory: str, *frames: bytes) -> Path:
    path = Path(directory) / "run.syscap"
    header = struct.pack("<I", BENCHMARK.CAPTURE_MAGIC)
    header += b"\0" * (BENCHMARK.CAPTURE_HEADER_SIZE - len(header))
    path.write_bytes(header + b"".join(frames))
    return path


class CaptureTests(unittest.TestCase):
    def test_reads_marks_and_skips_other_frames(self):
        with tempfile.TemporaryDirectory() as directory:
            path = capture(
                directory,
                frame(1, 10),
                mark(20, 5 * MS, "GTK", "Frameclock cycle", "frame"),
            )
            self.assertEqual(
                list(BENCHMARK.read_marks(path)),
                [(20, 5 * MS, "GTK", "Frameclock cycle", "frame")],
            )

    def test_rejects_files_that_are_not_captures(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.syscap"
            path.write_bytes(b"\0" * BENCHMARK.CAPTURE_HEADER_SIZE)
            with self.assertRaises(ValueError):
                list(BENCHMARK.read_marks(path))

    def test_frame_summary_counts_only_gtk_marks_inside_the_window(self):
        with tempfile.TemporaryDirectory() as directory:
            path = capture(
                directory,
                mark(5, 30 * MS, "GTK", "Frameclock cycle"),
                mark(100, 5 * MS, "GTK", "Frameclock cycle"),
                mark(200, 10 * MS, "GTK", "Frameclock cycle"),
                mark(300, 20 * MS, "GTK", "Frameclock cycle"),
                mark(310, 2 * MS, "GTK", "Validate CSS"),
                mark(320, 7 * MS, "GLib", "Validate CSS"),
                mark(2_000, 4 * MS, "GTK", "Validate CSS"),
            )
            summary = BENCHMARK.frame_summary(path, 100, 1_000)

        self.assertEqual(summary["frames"], 3)
        self.assertEqual(summary["frames_over_budget"], 1)
        self.assertEqual(summary["frame_p50_ms"], 10)
        self.assertEqual(summary["frame_max_ms"], 20)
        self.assertEqual(summary["css_ms"], 2)


class ReportTests(unittest.TestCase):
    def test_summary_reports_fastest_and_median_cpu_per_step(self):
        run = dict.fromkeys((key for key, _ in BENCHMARK.REPORTED), 0.0)
        runs = [
            {**run, "mode": "list", "fixture": "mixed", "label": "after",
             "main_cpu_per_step_ms": cpu}
            for cpu in (4.0, 6.0, 9.0)
        ]
        row = BENCHMARK.summarize(runs).splitlines()[-1]
        self.assertTrue(row.startswith("| list | mixed | after | 3 | 4.00 | 6.00 |"), row)


class ArgumentTests(unittest.TestCase):
    def test_preferences_keep_booleans_and_numbers(self):
        self.assertEqual(
            BENCHMARK.parse_preference("columns_show_sizes=false"),
            ("columns_show_sizes", False),
        )
        self.assertEqual(BENCHMARK.parse_preference("text_scale=2"), ("text_scale", 2))
        self.assertEqual(BENCHMARK.parse_preference("theme=azure-glow"), ("theme", "azure-glow"))
        with self.assertRaises(argparse.ArgumentTypeError):
            BENCHMARK.parse_preference("columns_show_sizes")

    def test_lists_reject_unknown_names(self):
        self.assertEqual(BENCHMARK.parse_list("list,icons", BENCHMARK.MODES), ["list", "icons"])
        with self.assertRaises(argparse.ArgumentTypeError):
            BENCHMARK.parse_list("columns,grid", BENCHMARK.MODES)


if __name__ == "__main__":
    unittest.main()
