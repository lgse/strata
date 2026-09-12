#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Private-display evidence for a supplied build; not a replacement for canonical E2E.

Use generated media only. The default is video-only without audio access.
Speaker output requires --audio-runtime and explicit --allow-audible consent;
the GUI still uses a private display and buses. No audio recording is performed.
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


def resources(pid):
    pending, found = [pid], set()
    rss = pss = helpers = decoders = unreadable = 0
    while pending:
        current = pending.pop()
        if current in found:
            continue
        found.add(current)
        process = Path(f"/proc/{current}")
        try:
            argv = (process / "cmdline").read_bytes().split(b"\0")
            program = Path(os.fsdecode(argv[0])).name
            helpers += int(program == "strata-media-helper" or
                           (program == "strata" and b"--preview-helper" in argv))
            decoders += int(program in ("ffmpeg", "ffprobe"))
            for task in (process / "task").iterdir():
                try:
                    pending.extend(int(value) for value in (task / "children").read_text().split())
                except FileNotFoundError:
                    pass
            try:
                values = dict(re.findall(r"^(Rss|Pss):\s+(\d+)", (process / "smaps_rollup").read_text(), re.M))
                rss += int(values.get("Rss", 0))
                pss += int(values.get("Pss", 0))
            except PermissionError:
                unreadable += 1
            except FileNotFoundError:
                pass
        except FileNotFoundError:
            pass
    process = Path(f"/proc/{pid}")
    return dict(app_fd=len(list((process / "fd").iterdir())),
                app_threads=len(list((process / "task").iterdir())),
                app_rss_kib=int((process / "statm").read_text().split()[1]) * os.sysconf("SC_PAGE_SIZE") // 1024,
                tree_rss_kib=rss, tree_pss_kib=pss, unreadable_smaps=unreadable,
                media_helpers=helpers, decoders=decoders)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expect-error")
    parser.add_argument("--cycles", type=int, default=0)
    parser.add_argument("--pause-release", action="store_true")
    parser.add_argument("--audio-fixture", choices=("audio", "av"))
    parser.add_argument("--audio-runtime", type=Path)
    parser.add_argument("--allow-audible", action="store_true")
    args = parser.parse_args()
    if args.audio_runtime and (not args.allow_audible or not args.audio_fixture):
        parser.error("external audio runtime requires an audio fixture and explicit --allow-audible consent")
    if not 0 <= args.cycles <= 1000:
        parser.error("cycles must be 0..1000")
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    path = os.environ.get("PATH", os.defpath)
    os.environ.clear()
    os.environ.update(PATH=path, STRATA_BINARY=str(binary))
    output.mkdir(parents=True, exist_ok=True)
    fixtures = output / "fixtures"
    fixtures.mkdir(exist_ok=True)
    clip = fixtures / ("clip.wav" if args.audio_fixture == "audio" else "clip.mkv")
    if not clip.exists():
        command = ["/usr/bin/ffmpeg", "-nostdin", "-v", "error"]
        duration = 4 if args.audio_fixture else 30
        if args.audio_fixture != "audio":
            command += ["-f", "lavfi", "-i", f"testsrc2=size=160x90:rate=30:duration={duration}"]
        if args.audio_fixture:
            command += ["-f", "lavfi", "-i", "sine=frequency=660:sample_rate=48000:duration=4", "-c:a", "pcm_s16le"]
        if args.audio_fixture != "audio":
            command += ["-c:v", "ffv1"]
        subprocess.run(command + ["-threads", "1", str(clip)], check=True, timeout=60)
    (fixtures / "notes.txt").write_text("Browsing and text previews remain usable.\n")
    sys.path.insert(0, str(ROOT / "tests/e2e"))
    from harness.display import HeadlessDisplay
    from harness.environment import TestEnvironment
    display, home = HeadlessDisplay(), TestEnvironment()
    app = connection = None
    results = dict(binary=str(binary), cycles=args.cycles, audio="not exercised: video-only fixture", completed=False)
    try:
        display.start()
        os.environ.update(home.variables())
        os.environ.update(display.environment)
        from harness.application import Application
        from harness.browser import Strata
        from harness.fixtures import FixtureTree
        from harness.interaction import Keyboard, Pointer
        from harness.xtest import XTestConnection
        from harness import tree
        tree.connect()
        home.write_preferences({"video_preview_backend": "software", "preview_muted": not bool(args.audio_runtime), "preview_volume": 0.15})
        app_display = display
        if args.audio_runtime:
            # Redirect only the app's audio runtime, retaining its private GUI buses.
            from types import SimpleNamespace
            app_display = SimpleNamespace(environment=dict(display.environment, XDG_RUNTIME_DIR=str(args.audio_runtime.resolve(strict=True))))
        app = Application(app_display, home, fixtures).start()
        connection = XTestConnection(display.display)
        browser = Strata(app, Keyboard(connection), Pointer(connection), FixtureTree(fixtures), home, display)
        browser.select_entry_with_keyboard(clip.name)
        start = time.monotonic()
        browser.keyboard.press("space")
        if args.expect_error:
            browser.wait(lambda: browser.preview_shows(args.expect_error), "actionable dependency guidance", timeout=25)
            browser.screenshot(output / "unavailable.png")
            browser.keyboard.press("Escape")
            browser.wait(lambda: browser.preview() is None, "close unavailable preview")
            browser.select_entry("notes.txt")
            browser.keyboard.press("space")
            browser.wait(lambda: browser.preview_shows("Browsing and text previews remain usable."), "unrelated text preview")
            browser.screenshot(output / "text-still-usable.png")
            results.update(expected_error=args.expect_error, text_preview_usable=True)
        elif args.audio_fixture:
            browser.wait(lambda: browser.preview_shows("0:01/0:04"), "native audio clock progress", timeout=25)
            results.update(audio=args.audio_fixture, configured_volume=0.15, playing=resources(app.process.popen.pid))
            browser.screenshot(output / "audio-playing.png")
            browser.wait(lambda: browser.preview_shows("0:04/0:04"), "native audio EOS", timeout=15)
            browser.keyboard.press("Escape")
            browser.wait(lambda: browser.preview() is None, "close audio preview")
            results["settled"] = resources(app.process.popen.pid)
        else:
            browser.wait(lambda: browser.preview_shows("0:01/"), "one-second playback", timeout=25)
            results["one_second_label_wall_ms"] = round((time.monotonic() - start) * 1000, 2)
            browser.screenshot(output / "playing.png")
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
                before, start = seconds(), time.monotonic()
                browser.keyboard.press("Right")
                browser.wait(lambda: seconds() >= before + 4, "paused seek acknowledgement", timeout=25)
                seeks.append(round((time.monotonic() - start) * 1000, 2))
            results["paused_seek_ms"] = seeks
            pid = app.process.popen.pid
            if args.pause_release:
                results["paused_before_release"] = resources(pid)
                position = seconds()
                time.sleep(31)
                results["paused_after_31s"] = resources(pid)
                assert results["paused_after_31s"]["media_helpers"] == 0
                assert results["paused_after_31s"]["decoders"] == 0
                assert seconds() == position
                browser.pointer.click(pause)
                browser.wait(lambda: seconds() > position, "resume at retained position", timeout=25)
            browser.keyboard.press("Escape")
            browser.wait(lambda: browser.preview() is None, "close preview")
            samples = []
            for cycle in range(args.cycles):
                browser.select_entry("clip.mkv")
                browser.keyboard.press("space")
                browser.wait(lambda: browser.preview_shows("0:00/0:30"), "prepared generation", timeout=25)
                time.sleep(0.15)
                browser.keyboard.press("Escape")
                browser.wait(lambda: browser.preview() is None, "cycle close")
                browser.wait(lambda: resources(pid)["media_helpers"] == 0 and resources(pid)["decoders"] == 0,
                             "native descendants reaped", timeout=5)
                samples.append(dict(resources(pid), cycle=cycle + 1))
            results["closed_cycles"] = samples
            time.sleep(2)
            results["settled"] = resources(pid)
        results["completed"] = True
        print(json.dumps(results, indent=2), flush=True)
    finally:
        (output / "runtime.json").write_text(json.dumps(results, indent=2) + "\n")
        if app is not None:
            (output / "application.log").write_text(app.log())
            app.stop()
        if connection is not None:
            connection.close()
        home.cleanup()
        display.stop()


if __name__ == "__main__":
    main()
