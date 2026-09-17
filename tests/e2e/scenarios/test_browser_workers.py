# SPDX-License-Identifier: MIT
"""Real browser requests reuse bounded sandbox supervisors across navigation."""

import hashlib
import os
import re
import signal
import subprocess
from pathlib import Path

import pytest
from PIL import Image

from harness.modes import ALL_MODES


@pytest.fixture
def test_environment(test_environment, monkeypatch):
    variables = test_environment.variables
    monkeypatch.setattr(test_environment, "variables", lambda: {
        **variables(),
        "STRATA_THUMBNAIL_WORKERS": "2",
        "RUST_LOG": "strata=info,strata::sandbox::browser=debug",
    })
    return test_environment


@pytest.fixture
def fixture_tree(fixture_tree):
    for folder in ["photos-a", "photos-b", "photos-c"]:
        directory = fixture_tree.path(folder)
        directory.mkdir()
        for index in range(6):
            Image.new("RGB", (320 + index, 180), (40, 160, 80)).save(directory / f"photo-{index}.png")
    return fixture_tree


@pytest.mark.parametrize("mode", ALL_MODES)
def test_browser_workers_reuse_processes_and_preserve_source_details(strata, mode, test_environment):
    strata.switch_view(mode)

    def cached(folder):
        bucket = test_environment.cache_home / "thumbnails" / "large"
        return all((bucket / (hashlib.md5(path.as_uri().encode()).hexdigest() + ".png")).is_file()
                   for path in strata.fixture.path(folder).glob("*.png"))

    def starts():
        return strata.application.log().count("browser sandbox started")

    strata.open_directory("photos-a")
    strata.wait(lambda: cached("photos-a"), "first folder thumbnails persisted")
    initial = starts()
    persistent = "Landlock ABI 3 unavailable" not in strata.application.log()
    assert 1 <= initial <= 2 if persistent else initial == 0
    if mode == "Icons":
        strata.wait(lambda: strata.window.find(role="label", name="320×180"), "original image dimensions")
    strata.keyboard.press("alt+Left")
    strata.wait(lambda: strata.entry("photos-b"), "parent folder")
    strata.open_directory("photos-b")
    strata.wait(lambda: cached("photos-b"), "second folder thumbnails persisted")
    assert starts() <= 2, "navigation must reuse the process-wide pool"
    assert strata.application.log().count("browser worker completed") >= 12
    if persistent:
        log = re.sub(r"\x1b\[[0-9;]*m", "", strata.application.log())
        pid = int(re.findall(r"browser sandbox started pid=(\d+)", log)[-1])
        status = Path(f"/proc/{pid}/status").read_text()
        assert f"PPid:\t{strata.application.process.popen.pid}\n" in status
        os.kill(pid, signal.SIGKILL)
        strata.keyboard.press("alt+Left")
        strata.wait(lambda: strata.entry("photos-c"), "parent after worker loss")
        strata.open_directory("photos-c")
        strata.wait(lambda: cached("photos-c"), "replacement worker to finish the next folder")
        assert starts() == initial + 1
    for path in strata.fixture.path("photos-a").glob("*.png"):
        with Image.open(path) as source:
            assert source.getpixel((0, 0)) == (40, 160, 80)


@pytest.mark.preferences(browser_mode="list")
def test_icons_fills_duration_without_regenerating_cached_thumbnails(strata, test_environment):
    directory = strata.fixture.path("media-details")
    directory.mkdir()
    Image.new("RGB", (640, 360), "green").save(directory / "photo.png")
    Image.new("RGB", (400, 200), "blue").save(directory / "page.pdf")
    (directory / "shape.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="256" height="128">'
        '<rect width="256" height="128" fill="green"/></svg>'
    )
    for source, arguments, name in [
        ("color=c=blue:s=320x180:r=12:d=2", ["-c:v", "libx264", "-pix_fmt", "yuv420p"], "clip.mp4"),
        ("sine=frequency=440:duration=3", ["-c:a", "pcm_s16le"], "sound.wav"),
    ]:
        subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", source,
                        *arguments, "-threads", "1", str(directory / name)], check=True, timeout=30)
    strata.open_directory("media-details")
    key = hashlib.md5((directory / "clip.mp4").as_uri().encode()).hexdigest()
    cached = test_environment.cache_home / "thumbnails" / "large" / f"{key}.png"
    for source in ["clip.mp4", "photo.png", "page.pdf", "shape.svg"]:
        digest = hashlib.md5((directory / source).as_uri().encode()).hexdigest()
        output = cached.parent / f"{digest}.png"
        strata.wait(output.is_file, f"sandboxed thumbnail for {source}")
    stored = cached.stat().st_mtime_ns
    strata.switch_view("Icons")
    for caption in ["0:02", "0:03", "640×360", "256×128"]:
        strata.wait(lambda: strata.window.find(role="label", name=caption), f"source detail {caption}")
    assert cached.stat().st_mtime_ns == stored
    assert strata.application.log().count("browser sandbox started") <= 2
