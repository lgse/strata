# SPDX-License-Identifier: MIT
from hashlib import md5
import shutil
import subprocess

from PIL import Image, PngImagePlugin
import pytest

from harness.fixtures import FixtureTree


@pytest.fixture
def fixture_tree(test_environment, request):
    fixture = FixtureTree.create({})
    source = fixture.path("small.png")
    Image.new("RGB", (80, 40), "green").save(source)
    extension = getattr(request, "param", None)
    if extension == "heic":
        converter = shutil.which("magick") or shutil.which("convert")
        assert converter is not None, "the pinned image must provide ImageMagick"
        subprocess.run(
            [converter, "-limit", "thread", "1", str(source), str(fixture.path("camera.heic"))],
            check=True, capture_output=True, timeout=30,
        )
    elif extension in {"mov", "mp4"}:
        subprocess.run(
            ["ffmpeg", "-hide_banner", "-loglevel", "error", "-loop", "1", "-i", str(source),
             "-t", "4", "-c:v", "libx264", "-threads", "1", "-pix_fmt", "yuv420p", "-an",
             str(fixture.path(f"camera.{extension}"))],
            check=True, capture_output=True, timeout=30,
        )
    cache = test_environment.cache_home / "thumbnails" / "large"
    cache.mkdir(parents=True, mode=0o700)
    metadata = PngImagePlugin.PngInfo()
    metadata.add_text("Thumb::URI", source.as_uri())
    metadata.add_text("Thumb::MTime", str(int(source.stat().st_mtime)))
    name = md5(source.as_uri().encode(), usedforsecurity=False).hexdigest() + ".png"
    Image.new("RGB", (256, 128), "blue").save(cache / name, pnginfo=metadata)
    try:
        yield fixture
    finally:
        fixture.cleanup()


@pytest.mark.parametrize("fixture_tree", ["heic", "mov", "mp4"], indirect=True)
def test_camera_formats_render_through_preview_sandbox(strata, request):
    extension = request.node.callspec.params["fixture_tree"]
    strata.select_entry_with_keyboard(f"camera.{extension}")
    strata.keyboard.press("space")
    if extension == "heic":
        def rendered():
            preview = strata.preview()
            return preview is not None and any(
                node.screen_bounds().width >= 80 for node in preview.find_all(role="image")
            )
        strata.wait(rendered, "the HEIC image decoded by the preview sandbox")
    else:
        strata.wait(lambda: strata.preview_shows("/0:04"), "the sandboxed video duration")
    assert not strata.preview_shows("Preview unavailable")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is None, "the preview to close")


def test_small_image_preview_never_uses_an_upscaled_thumbnail_as_its_native_size(strata):
    strata.select_entry_with_keyboard("small.png")
    strata.keyboard.press("space")
    observed = []

    def rendered():
        preview = strata.preview()
        if preview is None:
            return False
        images = [
            node.screen_bounds()
            for node in preview.find_all(role="image")
            if node.screen_bounds().width > 40
        ]
        if not images:
            return False
        bounds = max(images, key=lambda b: b.width * b.height)
        observed.append((bounds.width, bounds.height))
        return bounds.width == 160 and bounds.height == 80

    strata.wait(rendered, "the native image rendered at no more than twice its original size")
    assert all(width <= 160 and height <= 80 for width, height in observed), observed
