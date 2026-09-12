#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Remove/restore runtime capabilities only inside an explicitly disposable container."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import sysconfig
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--uid", type=int, required=True)
    parser.add_argument("--gid", type=int, required=True)
    args = parser.parse_args()
    if os.geteuid() != 0 or args.uid <= 0:
        parser.error("requires container root and a non-root test owner")
    if not (Path("/run/.containerenv").exists() or Path("/.dockerenv").exists()):
        parser.error("refuse to mutate a host runtime")
    library = Path("/usr/lib") / sysconfig.get_config_var("MULTIARCH")
    cases = [
        ("missing-libraries", sorted(library.glob("libgst*.so.*")), "Media helper runtime libraries are missing", []),
        ("missing-ffmpeg", [Path("/usr/bin/ffmpeg"), Path("/usr/bin/ffprobe")], "Install FFmpeg", []),
        ("missing-sandbox", [Path("/usr/bin/bwrap")], "Install bubblewrap", []),
        ("missing-base-plugin", [library / "gstreamer-1.0/libgstapp.so"], "Audio output plugins are missing", ["--audio-fixture", "audio", "--private-pulse"]),
        ("missing-pulse-plugin", [library / "gstreamer-1.0/libgstpulseaudio.so"], "Audio output plugins are missing", ["--audio-fixture", "av", "--private-pulse"]),
        ("missing-server", [], "Audio output is unavailable", ["--audio-fixture", "audio"]),
    ]
    with tempfile.TemporaryDirectory(prefix="capability-control-") as temporary:
        control = Path(temporary)
        control.chmod(0o1777)
        for name, paths, message, extra in cases:
            assert paths or name == "missing-server"
            for path in paths:
                assert path.exists(), path
            ready, restored = control / f"{name}-ready", control / f"{name}-restored"
            command = ["/usr/bin/setpriv", f"--reuid={args.uid}", f"--regid={args.gid}", "--clear-groups",
                sys.executable, str(ROOT / "docs/evidence/850/runtime_evidence.py"),
                "--binary", str(args.bundle / "strata"), "--output", str(args.output / name), "--expect-error", message, *extra]
            subprocess.run(command + ["--prepare-only"], check=True, timeout=90)
            hidden = control / name
            hidden.mkdir(mode=0o700)
            for path in paths:
                shutil.move(path, hidden / path.name)
            process = None
            try:
                if paths:
                    command += ["--restore-ready", str(ready), "--restored", str(restored)]
                process = subprocess.Popen(command)
                if paths:
                    deadline = time.monotonic() + 80
                    while not ready.exists():
                        assert process.poll() is None and time.monotonic() < deadline, name
                        time.sleep(0.02)
                    for path in paths:
                        shutil.move(hidden / path.name, path)
                    restored.touch()
                assert process.wait(timeout=90) == 0, name
            finally:
                for path in paths:
                    if (hidden / path.name).exists() or (hidden / path.name).is_symlink():
                        shutil.move(hidden / path.name, path)
                if process is not None and process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == "__main__":
    main()
