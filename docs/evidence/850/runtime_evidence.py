#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Private-display evidence for a supplied build; not a replacement for canonical E2E.

Use generated media only. The default is video-only without audio access.
Speaker output requires --audio-runtime and explicit --allow-audible consent;
the GUI still uses a private display and buses. Private PulseAudio monitoring
records only this test's generated null-sink stream, never desktop audio.
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
    parser.add_argument("--restore-ready", type=Path)
    parser.add_argument("--restored", type=Path)
    parser.add_argument("--cycles", type=int, default=0)
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--pause-release", action="store_true")
    parser.add_argument("--activate-archive", type=Path)
    parser.add_argument("--launcher", type=Path)
    parser.add_argument("--audio-fixture", choices=("audio", "av"))
    parser.add_argument("--audio-runtime", type=Path)
    parser.add_argument("--private-pulse", action="store_true")
    parser.add_argument("--allow-audible", action="store_true")
    args = parser.parse_args()
    if args.private_pulse and (args.audio_runtime or not args.audio_fixture):
        parser.error("private PulseAudio requires an audio fixture and cannot use an external runtime")
    if args.audio_fixture and (args.cycles or args.pause_release):
        parser.error("audio smoke and video ownership measurements are separate runs")
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
        duration = (8 if args.private_pulse else 4) if args.audio_fixture else 30
        if args.audio_fixture != "audio":
            command += ["-f", "lavfi", "-i", f"testsrc2=size=160x90:rate=30:duration={duration}"]
        if args.audio_fixture:
            command += ["-f", "lavfi", "-i", f"sine=frequency=660:sample_rate=48000:duration={duration}", "-c:a", "pcm_s16le"]
        if args.audio_fixture != "audio":
            command += ["-c:v", "ffv1"]
        subprocess.run(command + ["-threads", "1", str(clip)], check=True, timeout=60)
    (fixtures / "notes.txt").write_text("Browsing and text previews remain usable.\n")
    if args.prepare_only:
        return
    sys.path.insert(0, str(ROOT / "tests/e2e"))
    from harness.display import HeadlessDisplay
    from harness.environment import TestEnvironment
    display, home = HeadlessDisplay(), TestEnvironment()
    app = connection = pulse = monitor = None
    audio_logs = []
    results = dict(binary=str(binary), cycles=args.cycles, fixture=args.audio_fixture or "video", audio="not exercised", completed=False)
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
        if args.private_pulse:
            runtime = display.runtime_dir / "pulse"
            runtime.mkdir(mode=0o700)
            native = runtime / "native"
            pulse_log = (output / "pulse.log").open("wb")
            monitor_log = (output / "private-monitor.raw").open("wb")
            audio_logs.extend([pulse_log, monitor_log])
            pulse = subprocess.Popen(["/usr/bin/pulseaudio", "--daemonize=no", "--exit-idle-time=-1",
                "--use-pid-file=no", "--realtime=no", "--high-priority=no", "--log-target=stderr", "-n",
                f"--load=module-native-protocol-unix socket={native}",
                "--load=module-null-sink sink_name=strata_test rate=48000 channels=2"],
                env=dict(os.environ, PULSE_RUNTIME_PATH=str(runtime)), stdout=subprocess.DEVNULL, stderr=pulse_log)
            deadline = time.monotonic() + 10
            while not native.is_socket():
                assert pulse.poll() is None and time.monotonic() < deadline, "private PulseAudio failed"
                time.sleep(0.02)
            monitor = subprocess.Popen(["/usr/bin/parec", f"--server=unix:{native}",
                "--device=strata_test.monitor", "--format=s16le", "--rate=48000", "--channels=2", "--raw"],
                env=os.environ, stdout=monitor_log, stderr=pulse_log)
            results["audio_service"] = "private PulseAudio with only a null sink; no physical output or recording of other audio"
        home.write_preferences({"video_preview_backend": "software", "preview_muted": not bool(args.audio_runtime or args.private_pulse), "preview_volume": 0.15})
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
            if args.restore_ready:
                assert args.restored is not None
                args.restore_ready.touch()
                browser.wait(args.restored.exists, "dependency restoration", timeout=30)
                browser.keyboard.press("Escape")
                browser.wait(lambda: browser.preview() is None, "close text preview")
                browser.select_entry(clip.name)
                browser.keyboard.press("space")
                browser.wait(lambda: browser.preview_shows("0:01/"), "recovered without restarting the application", timeout=25)
                browser.screenshot(output / "restored.png")
                results["recovered_without_restart"] = True
        elif args.audio_fixture:
            duration = 8 if args.private_pulse else 4
            browser.wait(lambda: browser.preview_shows(f"0:01/0:{duration:02d}"), "native audio clock progress", timeout=25)
            results.update(audio=args.audio_fixture, configured_volume=0.15, playing=resources(app.process.popen.pid))
            browser.screenshot(output / "audio-playing.png")
            if args.private_pulse:
                import array
                def peak():
                    with (output / "private-monitor.raw").open("rb") as stream:
                        length = stream.seek(0, 2)
                        assert length > 48000 and length < 32 * 1024 * 1024
                        stream.seek(length - 48000)
                        samples = array.array("h", stream.read(48000))
                    return max(map(abs, samples))
                initial = peak()
                assert 200 < initial < 700, ("startup volume", initial)
                browser.keyboard.press("m")
                time.sleep(0.7)
                muted = peak()
                assert muted <= 2, ("live mute", muted)
                browser.keyboard.press("m")
                browser.keyboard.press("Up")
                time.sleep(0.7)
                louder = peak()
                assert 1.4 < louder / initial < 1.9, ("live volume", initial, louder)
                results["private_monitor_peaks"] = dict(startup=initial, muted=muted, louder=louder)
            browser.wait(lambda: browser.preview_shows(f"0:{duration:02d}/0:{duration:02d}"), "native audio EOS", timeout=15)
            browser.keyboard.press("Escape")
            browser.wait(lambda: browser.preview() is None, "close audio preview")
            pid = app.process.popen.pid
            browser.wait(lambda: resources(pid)["media_helpers"] == 0 and resources(pid)["decoders"] == 0,
                         "audio descendants reaped", timeout=5)
            results["settled"] = resources(pid)
        else:
            browser.wait(lambda: browser.preview_shows("0:01/"), "one-second playback", timeout=25)
            results["one_second_label_wall_ms"] = round((time.monotonic() - start) * 1000, 2)
            if args.activate_archive:
                import tarfile
                assert args.launcher is not None
                with tarfile.open(args.activate_archive) as archive:
                    member = next(member for member in archive if member.name.endswith("/bundle.json"))
                    manifest = json.load(archive.extractfile(member))
                old_path = args.launcher.resolve(strict=True)
                subprocess.run(["/bin/bash", "-c", 'source "$1"; install_bundle "$2" "$3" "$4" "$5"', "activation",
                    str(ROOT / "install.sh"), str(args.activate_archive), manifest["release_tag"][1:], manifest["target"], str(args.launcher)],
                    env={"PATH": "/usr/bin:/bin", "STRATA_INSTALLER_TESTING": "1"}, check=True,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
                assert args.launcher.resolve(strict=True) != old_path and old_path.is_file()
                browser.keyboard.press("Escape")
                browser.wait(lambda: browser.preview() is None, "close old generation")
                browser.select_entry(clip.name)
                browser.keyboard.press("space")
                browser.wait(lambda: browser.preview_shows("0:01/"), "old process starts a matching helper after activation", timeout=25)
                assert not app.process.exited()
                results["old_process_reopened_after_activation"] = True
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
        for process in (monitor, pulse):
            if process is not None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        for stream in audio_logs:
            stream.close()
        home.cleanup()
        display.stop()


if __name__ == "__main__":
    main()
