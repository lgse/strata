# SPDX-License-Identifier: MIT
import subprocess

import pytest

from harness.fixtures import FixtureTree


@pytest.fixture
def fixture_tree():
    fixture = FixtureTree.create({"notes.txt": "plain"})
    for name, duration in [("alpha.mp4", 6), ("beta.mp4", 5)]:
        subprocess.run(
            ["ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
             f"testsrc2=size=320x180:rate=24:duration={duration}", "-c:v", "libx264",
             "-threads", "1", "-pix_fmt", "yuv420p", "-an", str(fixture.path(name))],
            check=True, capture_output=True, timeout=60,
        )
    try:
        yield fixture
    finally:
        fixture.cleanup()


def test_video_preview_shows_badges_and_steps_to_the_next_video(strata):
    strata.select_entry_with_keyboard("alpha.mp4")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("1 of 2 in folder"), "the video view")
    strata.wait(lambda: strata.preview_shows("H.264"), "the codec badge from the sandboxed probe")
    assert strata.preview_shows("24 fps")
    preview = strata.preview()
    assert preview.find(role="slider", name="Playback position") is not None
    assert preview.find(role="button", name="Next video") is not None
    assert not strata.preview_shows("Preview unavailable")

    strata.keyboard.press("ctrl+alt+shift+>")
    strata.wait(lambda: strata.preview_shows("2 of 2 in folder"), "the next video in the same view")
    strata.wait(lambda: strata.preview_shows("beta.mp4"), "the stepped file's title")
    assert not strata.preview_shows("Preview unavailable")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is None, "the preview to close")
