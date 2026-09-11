#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Private native evidence capture; not a replacement for canonical E2E.

Place executables at target/824-evidence/{before,after}-app/strata.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
OUTPUT = ROOT / "target/824-evidence"
SINK_RANKS = "pulsesink:0,pipewiresink:0,alsasink:0,osssink:0,oss4sink:0,jackaudiosink:0"


def resources(pid):
    pending, found = [pid], set()
    rss = pss = helpers = decoders = 0
    while pending:
        current = pending.pop()
        if current in found:
            continue
        found.add(current)
        root = Path(f"/proc/{current}")
        try:
            command = (root / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
            name = (root / "comm").read_text().strip()
            helpers += int("--preview-helper preview-media" in command and name == "strata")
            decoders += int(name == "ffmpeg")
            for task in (root / "task").iterdir():
                try:
                    pending.extend(int(value) for value in (task / "children").read_text().split())
                except (FileNotFoundError, PermissionError):
                    pass
            try:
                values = dict(re.findall(r"^(Rss|Pss):\s+(\d+)", (root / "smaps_rollup").read_text(), re.M))
                rss += int(values.get("Rss", 0))
                pss += int(values.get("Pss", 0))
            except (FileNotFoundError, PermissionError):
                pass
        except (FileNotFoundError, PermissionError):
            pass
    root = Path(f"/proc/{pid}")
    return {
        "app_fd": len(list((root / "fd").iterdir())),
        "app_threads": len(list((root / "task").iterdir())),
        "app_rss_kib": int((root / "statm").read_text().split()[1]) * os.sysconf("SC_PAGE_SIZE") // 1024,
        "tree_rss_kib": rss,
        "tree_pss_kib": pss,
        "media_helpers": helpers,
        "ffmpeg": decoders,
    }


def fixtures():
    directory = OUTPUT / "fixtures"
    directory.mkdir(parents=True, exist_ok=True)
    video = directory / "clip.mp4"
    if not video.exists():
        subprocess.run([
            "ffmpeg", "-nostdin", "-v", "error", "-f", "lavfi", "-i",
            "testsrc2=size=1920x1080:rate=30:duration=30", "-f", "lavfi", "-i",
            "sine=frequency=660:sample_rate=48000:duration=30", "-c:v", "libx264",
            "-preset", "ultrafast", "-threads", "2", "-c:a", "aac", str(video),
        ], check=True, timeout=90)
    (directory / "notes.txt").write_text("Selection cancels media decoding.\n")
    return directory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", choices=("before", "after"))
    parser.add_argument("--cycles", type=int, help="profile without the 31-second pause test")
    parser.add_argument("--two-arenas", action="store_true", help="diagnostic allocator override in this child only")
    args = parser.parse_args()
    if args.cycles is not None and not 1 <= args.cycles <= 1000:
        parser.error("cycles must be 1..1000")
    if args.two_arenas and args.cycles is None:
        parser.error("--two-arenas requires --cycles")
    profile = args.cycles is not None
    tag = args.version + (f"-profile-{args.cycles}" if profile else "") + ("-arenas2" if args.two_arenas else "")
    path = os.environ.get("PATH", os.defpath)
    os.environ.clear()
    os.environ.update({"PATH": path, "DBUS_SESSION_BUS_ADDRESS": "unix:path=/tmp/strata-824-no-session"})
    sys.path.insert(0, str(ROOT / "tests/e2e"))
    from harness.display import HeadlessDisplay
    from harness.environment import TestEnvironment

    directory = fixtures()
    display, home = HeadlessDisplay(), TestEnvironment()
    app = connection = recording = None
    try:
        display.start()
        os.environ.update(home.variables())
        os.environ.update(display.environment)
        os.environ["STRATA_BINARY"] = str(OUTPUT / f"{args.version}-app/strata")
        from harness.application import Application
        from harness.browser import Strata
        from harness.fixtures import FixtureTree
        from harness.interaction import Keyboard, Pointer
        from harness.xtest import XTestConnection
        from harness import tree

        tree.connect()
        home.write_preferences({"video_preview_backend": "software", "preview_muted": True})
        original_variables = home.variables
        home.variables = lambda: {
            **original_variables(),
            **({"MALLOC_ARENA_MAX": "2"} if args.two_arenas else {}),
            "RUST_LOG": "strata=debug,strata::ui::media::diagnostics=trace",
            "GST_PLUGIN_FEATURE_RANK": SINK_RANKS,
        }
        app = Application(display, home, directory).start()
        connection = XTestConnection(display.display)
        browser = Strata(app, Keyboard(connection), Pointer(connection), FixtureTree(directory), home, display)
        browser.select_entry_with_keyboard("clip.mp4")
        recording = subprocess.Popen([
            "ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "x11grab", "-framerate", "20",
            "-video_size", "1440x900", "-i", display.display, "-t", "6", "-c:v", "libx264",
            "-preset", "ultrafast", "-threads", "2", str(OUTPUT / f"{tag}-startup.mp4"),
        ], env={"PATH": path, **display.environment})
        time.sleep(0.25)
        started = time.monotonic()
        browser.keyboard.press("space")
        browser.wait(lambda: browser.preview_shows("0:01/"), "one-second playback label", timeout=25)
        results = {"one_second_label_wall_s": time.monotonic() - started}
        browser.screenshot(OUTPUT / f"{tag}.png")
        recording.wait(timeout=10)
        recording = None
        pause = browser.preview().find(role="button", name="Play/Pause (Space)")
        assert pause is not None
        browser.pointer.click(pause)
        label = next(node for node in browser.preview().find_all(role="label")
                     if re.match(r"^\d+:\d+/\d+:\d+$", node.name))

        def seconds():
            minutes, seconds = label.name.split("/")[0].split(":")
            return int(minutes) * 60 + int(seconds)

        seeks = []
        for _ in range(3):
            before = seconds()
            started = time.monotonic()
            browser.keyboard.press("Right")
            browser.wait(lambda: seconds() >= before + 4, "paused seek acknowledgement", timeout=25)
            seeks.append(round((time.monotonic() - started) * 1000, 2))
        results["paused_keyboard_seek_ms"] = seeks
        pid = app.process.popen.pid
        if args.version == "after" and not profile:
            results["paused_before_release"] = resources(pid)
            position = seconds()
            time.sleep(31)
            results["paused_after_31s"] = resources(pid)
            assert results["paused_after_31s"]["media_helpers"] == 0
            assert results["paused_after_31s"]["ffmpeg"] == 0
            assert seconds() == position
            browser.pointer.click(pause)
            browser.wait(lambda: seconds() > position, "idle resume at retained position", timeout=25)
            results["after_idle_resume"] = resources(pid)
        if args.version == "after":
            browser.keyboard.press("Escape")
            browser.wait(lambda: browser.preview() is None, "close preview")
            samples = []
            for iteration in range(args.cycles or 20):
                browser.select_entry("clip.mp4")
                browser.keyboard.press("space")
                browser.wait(lambda: browser.preview_shows("0:00/0:30"), "stream prepared", timeout=25)
                time.sleep(0.12)
                browser.keyboard.press("Escape")
                browser.wait(lambda: browser.preview() is None, "cycle close")
                time.sleep(0.15)
                sample = {**resources(pid), "cycle": iteration + 1}
                samples.append(sample)
                if profile and iteration in (4, 19, 49, 99):
                    (OUTPUT / f"{tag}-{iteration + 1}.smaps").write_text(Path(f"/proc/{pid}/smaps").read_text())
                assert sample["media_helpers"] == 0 and sample["ffmpeg"] == 0, sample
            results["closed_cycles"] = samples
            if profile:
                time.sleep(20)
                results["settled"] = resources(pid)
                (OUTPUT / f"{tag}-settled.smaps").write_text(Path(f"/proc/{pid}/smaps").read_text())
        (OUTPUT / f"{tag}-native.log").write_text(app.log())
        (OUTPUT / f"{tag}-native.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(results, indent=2), flush=True)
    except BaseException:
        log = home.root / "strata.log"
        if log.exists():
            print(log.read_text()[-10000:], flush=True)
        raise
    finally:
        if recording is not None:
            recording.terminate()
            recording.wait(timeout=3)
        if app is not None:
            app.stop()
        if connection is not None:
            connection.close()
        home.cleanup()
        display.stop()


if __name__ == "__main__":
    main()
