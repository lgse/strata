# SPDX-License-Identifier: MIT
"""Real browser requests reuse bounded sandbox supervisors across navigation."""

import hashlib
import os
import re
import signal
import subprocess
from pathlib import Path

import pytest
from PIL import Image, PngImagePlugin

from harness.modes import ALL_MODES


@pytest.fixture
def worker_idle_seconds():
    return 60


@pytest.fixture
def test_environment(test_environment, monkeypatch, worker_idle_seconds):
    variables = test_environment.variables
    monkeypatch.setattr(test_environment, "variables", lambda: {
        **variables(),
        "STRATA_THUMBNAIL_WORKERS": "2",
        "STRATA_THUMBNAIL_IDLE_SECONDS": str(worker_idle_seconds),
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


def _folder_cached(strata, test_environment, folder):
    bucket = test_environment.cache_home / "thumbnails" / "large"
    return all((bucket / (hashlib.md5(path.as_uri().encode()).hexdigest() + ".png")).is_file()
               for path in strata.fixture.path(folder).glob("*.png"))


def _worker_pids(strata):
    log = re.sub(r"\x1b\[[0-9;]*m", "", strata.application.log())
    return [int(pid) for pid in re.findall(r"browser sandbox started pid=(\d+)", log)]


def test_icons_rename_reuses_loaded_thumbnail_and_details(strata, test_environment):
    strata.switch_view("Icons")
    strata.open_directory("photos-a")
    strata.wait(lambda: _folder_cached(strata, test_environment, "photos-a"),
                "all original thumbnails persisted")
    for index in range(6):
        strata.wait(lambda index=index: strata.window.find(role="label", name=f"{320 + index}×180"),
                    "original source dimensions")
    strata.pointer.click(strata.entry("photo-0.png"))
    strata.keyboard.press("F2")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("photo-0-renamed.png")
    strata.wait(lambda: field.text == "photo-0-renamed.png", "replacement name")
    completed = strata.application.log().count("browser worker completed")
    strata.keyboard.press("Return")
    strata.wait_for_entry_gone("photo-0.png")
    strata.wait_for_selection(["photo-0-renamed.png"])
    strata.wait(lambda: strata.entry("photo-0-renamed.png").find(role="label", name="320×180"),
                "renamed image retains its dimensions")
    strata.settle(strata.entry("photo-0-renamed.png"))
    assert strata.application.log().count("browser worker completed") == completed


@pytest.mark.parametrize("mode", ALL_MODES)
def test_browser_workers_reuse_processes_and_preserve_source_details(strata, mode, test_environment):
    strata.switch_view(mode)

    def cached(folder):
        return _folder_cached(strata, test_environment, folder)

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
        pid = _worker_pids(strata)[-1]
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


@pytest.mark.parametrize("worker_idle_seconds", [1])
def test_idle_workers_exit_without_new_requests_and_restart_on_demand(strata, test_environment, worker_idle_seconds):
    strata.open_directory("photos-a")
    strata.wait(lambda: _folder_cached(strata, test_environment, "photos-a"), "initial thumbnails")
    original = _worker_pids(strata)
    persistent = "Landlock ABI 3 unavailable" not in strata.application.log()
    if persistent:
        assert original
        strata.wait(lambda: all(not Path(f"/proc/{pid}").exists() for pid in original)
                    and strata.application.log().count("idle browser sandbox retired") == len(original),
                    "idle sandbox processes to exit without another request")
    else:
        assert not original
    strata.keyboard.press("alt+Left")
    strata.wait(lambda: strata.entry("photos-b"), "parent after idle retirement")
    strata.open_directory("photos-b")
    strata.wait(lambda: _folder_cached(strata, test_environment, "photos-b"), "thumbnails after idle retirement")
    replacements = _worker_pids(strata)[len(original):]
    assert replacements if persistent else not replacements


@pytest.mark.preferences(browser_mode="icons")
def test_cached_icons_fill_dimensions_across_multiple_batches_after_a_jump(strata, test_environment):
    directory = strata.fixture.path("cached-photos")
    directory.mkdir()
    bucket = test_environment.cache_home / "thumbnails" / "large"
    bucket.mkdir(parents=True, exist_ok=True)
    for index in range(256):
        path = directory / f"photo-{index:04}.png"
        Image.new("RGB", (320 + index, 180), (40, 160, 80)).save(path)
        tags = PngImagePlugin.PngInfo()
        tags.add_text("Thumb::URI", path.as_uri())
        tags.add_text("Thumb::MTime", str(int(path.stat().st_mtime)))
        name = hashlib.md5(path.as_uri().encode()).hexdigest() + ".png"
        Image.new("RGB", (64, 32), (40, 160, 80)).save(bucket / name, pnginfo=tags)
    strata.open_directory("cached-photos")
    strata.select_entry("photo-0000.png")
    strata.keyboard.press("End")
    names = strata.wait(
        lambda: (names if len(names := strata.entry_names()) > 16
                 and "photo-0255.png" in names else None),
        "more than one batch of cached thumbnails in the final viewport",
    )
    for name in names:
        index = int(Path(name).stem.removeprefix("photo-"))
        caption = f"{320 + index}×180"
        strata.wait(lambda: strata.window.find(role="label", name=caption),
                    f"warm thumbnail source dimensions for {name}")


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
