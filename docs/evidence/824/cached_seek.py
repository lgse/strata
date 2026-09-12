#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""GtkMediaFile may open only benchmark.py's generated normalized clips."""
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
OUTPUT = ROOT / "target/824-evidence/benchmark"


def child(name, target):
    sys.path.insert(0, str(ROOT / "tests/e2e"))
    from harness.display import HeadlessDisplay
    from harness.environment import TestEnvironment
    display = HeadlessDisplay()
    home = TestEnvironment()
    try:
        display.start()
        os.environ.update(home.variables())
        os.environ.update(display.environment)
        os.environ.update({"GTK_A11Y": "none", "NO_AT_BRIDGE": "1",
                           "GST_PLUGIN_FEATURE_RANK": "pulsesink:0,pipewiresink:0,alsasink:0,osssink:0,oss4sink:0,jackaudiosink:0"})
        import gi
        gi.require_version("Gtk", "4.0")
        from gi.repository import Gtk, GLib
        Gtk.init()
        path = OUTPUT / f"legacy-{name}.mp4"
        assert name in {"short", "landscape", "portrait", "high-resolution", "variable-rate", "hour"}
        assert path.read_bytes()[4:8] == b"ftyp"
        media = Gtk.MediaFile.new_for_filename(str(path))
        media.set_muted(True)
        window = Gtk.Window()
        window.set_default_size(520, 800)
        window.set_child(Gtk.Picture.new_for_paintable(media))
        window.present()
        media.play()
        context = GLib.MainContext.default()

        def wait(predicate):
            deadline = time.monotonic() + 12
            while not predicate():
                assert media.get_error() is None, str(media.get_error())
                assert time.monotonic() < deadline, "legacy player deadline"
                context.iteration(False)
                time.sleep(0.0005)

        wait(lambda: media.is_prepared() and media.get_timestamp() > 100_000)
        media.pause()
        assert media.is_seekable()
        times = []
        for index in range(4):
            if index == 0:
                with path.open("rb") as source:
                    os.posix_fadvise(source.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
            started = time.monotonic()
            media.seek(target + (index % 2) * 33_333)
            wait(lambda: not media.is_seeking())
            times.append(round((time.monotonic() - started) * 1000, 2))
        print(json.dumps({"fixture": name, "cold_file_hint_ms": times[0],
                          "warm_median_ms": statistics.median(times[1:]), "runs_ms": times}), flush=True)
        media.clear()
        window.destroy()
        del window
        del media
    finally:
        home.cleanup()
        display.stop()


def main():
    path = os.environ.get("PATH", os.defpath)
    os.environ.clear()
    os.environ.update({"PATH": path, "DBUS_SESSION_BUS_ADDRESS": "unix:path=/tmp/strata-824-no-session"})
    if len(sys.argv) == 3:
        child(sys.argv[1], int(sys.argv[2]))
        return
    results = []
    for row in json.loads((OUTPUT / "timings.json").read_text()):
        run = subprocess.run([sys.executable, __file__, row["fixture"], str(row["seek_us"])],
                             text=True, capture_output=True, timeout=45)
        if run.returncode:
            raise RuntimeError(f"{row['fixture']}: {run.stderr}")
        result = json.loads(run.stdout)
        results.append(result)
        print(json.dumps(result), flush=True)
    (OUTPUT / "legacy-seeks.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
