#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original media must be parsed only inside bubblewrap."""
import argparse
import json
import os
from pathlib import Path
import select
import signal
import statistics
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
LIMIT = 30_000_000


def sandbox(binary, source, tick=None):
    args = ["bwrap", "--unshare-all", "--die-with-parent", "--new-session", "--clearenv",
            "--setenv", "PATH", "/usr/bin", "--setenv", "HOME", "/nonexistent",
            "--setenv", "XDG_CACHE_HOME", "/tmp/cache", "--proc", "/proc", "--dev", "/dev",
            "--size", "536870912", "--tmpfs", "/tmp", "--dir", "/app", "--dir", "/etc",
            "--ro-bind", "/usr", "/usr"]
    for path in ["/lib", "/lib64", "/etc/fonts", "/etc/ld.so.cache", "/etc/ImageMagick-7", "/etc/ImageMagick-6"]:
        args += ["--ro-bind-try", path, path]
    name = "/input" + source.suffix
    args += ["--ro-bind", str(binary), "/app/strata", "--ro-bind", str(source), name,
             "--", "/app/strata", "--preview-helper", "preview-media", name,
             "/dev/stdout", "520x800", "software"]
    if tick is not None:
        args.append(str(tick))
    return args


def read_bytes(process, count, deadline, allow_eof=False):
    data = bytearray()
    while len(data) < count:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
            raise TimeoutError("helper deadline")
        chunk = os.read(process.stdout.fileno(), min(count - len(data), 1 << 20))
        if not chunk:
            if allow_eof:
                return bytes(data)
            raise RuntimeError("truncated helper output")
        data.extend(chunk)
    return bytes(data)


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
    process.wait(timeout=5)


def render(binary, source, tick=None, moving=False):
    started = time.monotonic()
    process = subprocess.Popen(sandbox(binary, source, tick), stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL,
                               env={"PATH": os.environ["PATH"]}, start_new_session=True)
    try:
        deadline = started + 32
        if tick is None:
            data = bytearray()
            while True:
                chunk = read_bytes(process, 65536, deadline, allow_eof=True)
                data.extend(chunk)
                if len(data) > 32 * 1024 * 1024:
                    raise RuntimeError("legacy output limit")
                if len(chunk) < 65536:
                    break
            if process.wait(timeout=2) != 0 or data[4:8] != b"ftyp":
                raise RuntimeError("legacy sandbox failed (including installations affected by #806)")
            return time.monotonic() - started, bytes(data)
        magic, width, height, stride, audio, duration, start, reserved = struct.unpack(
            "<8sIIIIQII", read_bytes(process, 40, deadline))
        assert magic == b"STRRAW01" and 0 < width <= 520 and 0 < height <= 800
        assert stride == width * 4 and audio in (0, 1) and 0 < duration <= LIMIT
        assert start == tick and reserved == 0
        first = None
        for expected in range(tick, 900):
            kind, current, pts, video, pcm = struct.unpack("<IIQII", read_bytes(process, 24, deadline))
            assert kind == 1 and current == expected and pts == current * 1_000_000 // 30
            assert video == stride * height and pcm == (6400 if audio else 0)
            pixels = read_bytes(process, video, deadline)
            read_bytes(process, pcm, deadline)
            if not moving or (first is not None and first != pixels):
                return time.monotonic() - started, None
            first = pixels
        raise RuntimeError("fixture did not contain moving frames")
    finally:
        stop(process)


def cold_hint(path):
    with path.open("rb") as source:
        os.posix_fadvise(source.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)


def fixtures(directory):
    specifications = [
        ("short", "640x360", 30, 3, False, True),
        ("landscape", "1920x1080", 30, 30, False, True),
        ("portrait", "1080x1920", 30, 30, False, True),
        ("high-resolution", "3840x2160", 30, 10, False, True),
        ("variable-rate", "1280x720", 60, 30, True, True),
        ("hour", "64x48", 1, 3600, False, False),
    ]
    directory.mkdir(parents=True, exist_ok=True)
    for name, size, rate, duration, variable, audio in specifications:
        path = directory / f"{name}.mp4"
        if not path.exists():
            args = ["ffmpeg", "-nostdin", "-v", "error", "-f", "lavfi", "-i",
                    f"testsrc2=size={size}:rate={rate}:duration={duration}"]
            if audio:
                args += ["-f", "lavfi", "-i", f"sine=frequency=660:sample_rate=48000:duration={duration}"]
            if variable:
                args += ["-vf", r"select=if(lt(t\,3)\,1\,not(mod(n\,12)))", "-fps_mode", "vfr"]
            args += ["-c:v", "libx264", "-preset", "ultrafast", "-threads", "2", "-g", "60",
                     "-c:a", "aac", str(path)]
            subprocess.run(args, check=True, timeout=90, env={"PATH": os.environ["PATH"]})
        yield name, path, min(duration, 30) * 15


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True, type=Path, help="#823 executable (efe64e9)")
    parser.add_argument("--after", required=True, type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "target/824-evidence/benchmark")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    rows = []
    for name, source, seek_tick in fixtures(args.output / "fixtures"):
        row = {"fixture": name, "seek_us": seek_tick * 1_000_000 // 30}
        for label, binary, tick, moving in [
            ("legacy_conversion", args.before.resolve(), None, False),
            ("stream_first_motion_available", args.after.resolve(), 0, True),
            ("stream_seek_first_frame", args.after.resolve(), seek_tick, False),
        ]:
            times = []
            for trial in range(4):
                if trial == 0:
                    cold_hint(source)
                elapsed, data = render(binary, source.resolve(), tick, moving)
                times.append(round(elapsed * 1000, 2))
                if data is not None:
                    cached = args.output / f"legacy-{name}.mp4"
                    descriptor = os.open(cached, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o600)
                    with os.fdopen(descriptor, "wb") as output:
                        output.write(data)
            row[label] = {"cold_input_hint_ms": times[0], "warm_median_ms": statistics.median(times[1:]), "runs_ms": times}
        rows.append(row)
        print(json.dumps(row), flush=True)
    (args.output / "timings.json").write_text(json.dumps(rows, indent=2) + "\n")


if __name__ == "__main__":
    main()
