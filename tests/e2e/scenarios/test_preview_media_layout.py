# SPDX-License-Identifier: MIT
from hashlib import md5

from PIL import Image, PngImagePlugin
import pytest

from harness.fixtures import FixtureTree


@pytest.fixture
def fixture_tree(test_environment):
    fixture = FixtureTree.create({})
    source = fixture.path("small.png")
    Image.new("RGB", (80, 40), "green").save(source)
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
