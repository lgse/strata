# SPDX-License-Identifier: MIT
"""Properties inspects original media without starting preview playback."""

import subprocess

import pytest
from PIL import Image

from harness.raw_photo import write_raw_photo


def _media_fixture(tree):
    Image.new("RGB", (1600, 900), "green").save(tree.path("photo.png"))
    subprocess.run(
        [
            "ffmpeg", "-v", "error", "-y",
            "-f", "lavfi", "-i", "color=c=blue:s=320x180:r=24:d=2",
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=2",
            "-c:v", "libx264", "-c:a", "aac", "-ac", "2", "-shortest",
            str(tree.path("clip.mp4")),
        ],
        check=True,
        timeout=30,
    )
    subprocess.run(
        [
            "ffmpeg", "-v", "error", "-y", "-f", "lavfi",
            "-i", "sine=frequency=440:sample_rate=44100:duration=3",
            "-c:a", "pcm_s16le", str(tree.path("sound.wav")),
        ],
        check=True,
        timeout=30,
    )
    tree.path("broken.mp4").write_bytes(b"not a media container")
    write_raw_photo(tree.path("camera.DNG"))
    tree.path("broken.NEF").write_bytes(b"not a RAW image")


@pytest.fixture
def fixture_tree(fixture_tree):
    _media_fixture(fixture_tree)
    return fixture_tree


@pytest.mark.parametrize(
    "name, expected, absent",
    [
        ("photo.png", ["1600 × 900 pixels"], ["DURATION", "FRAME RATE", "AUDIO CODEC"]),
        (
            "clip.mp4",
            ["320 × 180 pixels", "0:00:02", "h264", "24.00 fps", "aac", "48.0 kHz", "2 (Stereo)", "BITRATE"],
            [],
        ),
        ("sound.wav", ["0:00:03", "pcm_s16le", "44.1 kHz", "1 (Mono)", "BITRATE"], ["RESOLUTION", "VIDEO CODEC"]),
        ("broken.mp4", ["Unavailable"], ["RESOLUTION", "DURATION"]),
    ],
)
def test_properties_reports_available_media_metadata(strata, fixture_tree, name, expected, absent):
    strata.open_context_menu(name)
    strata.choose_menu_item("Properties")
    dialog = strata.wait_for_dialog()
    for value in expected:
        strata.wait(
            lambda: dialog.find(role="label", name=value, rendered=False),
            f"Properties to report {value} for {name}",
        )
    for value in absent:
        assert dialog.find(role="label", name=value, rendered=False) is None
    if name == "clip.mp4":
        scroll = dialog.find(role="scroll pane")
        executable = dialog.find(name="Allow executing file as a program (+x)", rendered=False)
        assert scroll and executable
        strata.pointer.scroll(scroll.screen_bounds().center, clicks=20)

        def permission_is_reachable():
            control = executable.screen_bounds()
            viewport = scroll.screen_bounds()
            return viewport.y <= control.y and control.y + control.height <= viewport.y + viewport.height

        strata.wait(permission_is_reachable, "permissions to remain reachable below media details")
        strata.pointer.click(executable)
        strata.wait(
            lambda: fixture_tree.path(name).stat().st_mode & 0o111 == 0o111,
            "the scrolled permission control to remain usable",
        )
    close = dialog.find(role="button", name="Close dialog")
    assert close and close.is_rendered()
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "Properties to close after inspection")
    strata.open_context_menu("readme.md")
    strata.choose_menu_item("Properties")
    dialog = strata.wait_for_dialog()
    strata.wait(lambda: dialog.find(role="label", name="10 B"), "ordinary file properties")
    assert dialog.find(role="label", name="MEDIA") is None


@pytest.mark.parametrize("name", ["camera.DNG", "broken.NEF"])
@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_raw_details_match_in_preview_and_properties_and_clear_on_selection(strata, name):
    expected = {
        "DIMENSIONS": "600 × 400 pixels",
        "CAMERA": "Strata Test Camera",
        "LENS": "Synthetic 50mm lens",
        "FOCAL LENGTH": "50 mm",
        "SHUTTER SPEED": "1/250 s",
        "ISO": "400",
        "GPS COORDINATES": "-12.500000, -45.250000",
    }
    if name == "broken.NEF":
        expected = dict.fromkeys(expected, "N/A")

    def assert_details(surface):
        for field, value in expected.items():
            strata.wait(
                lambda: (label := surface.find(role="label", description=field, rendered=False))
                is not None and label.name == value,
                f"{field} to report {value} for {name}",
            )

    strata.select_entry(name)
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is not None, "RAW preview panel")
    assert_details(strata.preview())
    # Native-menu dispatch is covered by the quarantined #1154 regressions.
    strata.keyboard.press("alt+Return")
    dialog = strata.wait_for_dialog()
    assert_details(dialog)
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "Properties to close")
    strata.select_entry("photo.png")
    strata.wait(lambda: strata.preview_shows("photo.png"), "ordinary image preview")
    for field in expected:
        assert strata.preview().find(role="label", description=field, rendered=False) is None


@pytest.mark.preferences(browser_mode="icons", single_click_previews=False)
def test_media_details_are_properties_only(strata):
    strata.select_entry("clip.mp4")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("video/mp4"), "video preview ready")
    strata.open_context_menu("clip.mp4")
    strata.choose_menu_item("Properties")
    dialog = strata.wait_for_dialog()
    strata.wait(
        lambda: dialog.find(role="label", name="320 × 180 pixels", rendered=False),
        "source resolution in Properties",
    )
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "Properties to close")
    preview = strata.preview()
    assert preview is not None
    for description in ["Size", "Modified", "Type"]:
        value = preview.find(role="label", description=description, rendered=False)
        assert value is not None and value.name not in ("", "—")
    assert preview.find(role="label", name="video/mp4", rendered=False)
    for name in ["RESOLUTION", "DURATION", "BITRATE", "VIDEO CODEC", "FRAME RATE", "AUDIO CODEC", "SAMPLE RATE", "CHANNELS"]:
        assert preview.find(role="label", name=name, rendered=False) is None
