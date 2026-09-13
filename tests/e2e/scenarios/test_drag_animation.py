# SPDX-License-Identifier: MIT

import time

from PIL import ImageGrab
import pytest

from harness.environment import TestEnvironment as IsolatedEnvironment
from harness.interaction import MODIFIER_KEYSYMS, keysym
from harness.modes import ALL_MODES


@pytest.fixture
def test_environment():
    environment = IsolatedEnvironment()
    settings = environment.config_home / "gtk-4.0/settings.ini"
    settings.write_text(settings.read_text().replace(
        "gtk-enable-animations=false", "gtk-enable-animations=true"
    ))
    try:
        yield environment
    finally:
        environment.cleanup()


def text_position(image, bounds):
    """Measure glyph position independently of selection color and opacity."""
    pixels = image.convert("L").load()
    weights = []
    for y in range(bounds.y, bounds.y + bounds.height):
        row = [pixels[x, y] for x in range(bounds.x, bounds.x + bounds.width)]
        background = sorted(row)[len(row) // 2]
        weights.append(sum(abs(value - background) for value in row))
    contrast = sum(weights)
    assert contrast > 0, "the source label disappeared"
    return sum(y * weight for y, weight in enumerate(weights)) / contrast, contrast


@pytest.mark.preferences(reduce_motion=False)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_delete_keeps_survivors_in_place_until_dissolve_finishes(strata, mode):
    assert strata.view_mode() == mode
    strata.fixture.path("zz-survivor.txt").write_text("survivor")
    survivor = strata.entry("zz-survivor.txt")
    strata.select_entry("todo.txt")
    strata.settle(survivor)
    label = survivor.find(role="label", name="zz-survivor.txt")
    assert label is not None
    bounds = label.screen_bounds()
    grab = lambda: ImageGrab.grab(xdisplay=strata.display.display)
    original_y, _ = text_position(grab(), bounds)

    strata.keyboard.press("shift+Delete")
    strata.wait_for_dialog()
    strata.pointer.click(strata.dialog_button("Permanently delete 1 item"))
    strata.wait(
        lambda: not strata.fixture.path("todo.txt").exists(),
        "the file to be deleted",
    )
    strata.wait(lambda: strata.dialog() is None, "the delete dialog to disappear")
    frozen = grab()
    animated_y, _ = text_position(frozen, bounds)
    assert abs(animated_y - original_y) < 1, "surviving text moved before the dissolve finished"
    region = (bounds.x, bounds.y, bounds.x + bounds.width, bounds.y + bounds.height)
    frozen_pixels = frozen.crop(region).tobytes()
    strata.wait(
        lambda: grab().crop(region).tobytes() != frozen_pixels,
        "the updated layout to replace the frozen presentation",
    )
    final_label = strata.entry("zz-survivor.txt").find(role="label", name="zz-survivor.txt")
    assert final_label is not None and final_label.screen_bounds() != bounds


@pytest.mark.preferences(browser_mode="columns", reduce_motion=False)
@pytest.mark.parametrize("outcome", [
    "escape", "outside", "copy", "move", "noop", "failed",
    pytest.param("escape", marks=pytest.mark.preferences(reduce_motion=True), id="reduced-motion"),
])
def test_drag_completion_keeps_the_source_label_in_place(strata, outcome):
    fixture = strata.fixture
    original = fixture.path("todo.txt").read_bytes()
    source = strata.select_entry("todo.txt")
    label = source.find(role="label", name="todo.txt")
    assert label is not None
    bounds = label.screen_bounds()
    target = strata.entry("archive").screen_bounds().center
    if outcome in ("escape", "outside"):
        window = strata.window.screen_bounds()
        target = (window.x + window.width + 30, window.y + window.height + 20)
    elif outcome == "noop":
        pane = strata.pane().screen_bounds()
        target = (pane.x + pane.width // 2, pane.y + pane.height - 20)
    elif outcome == "failed":
        fixture.path("archive").chmod(0o555)

    connection = strata.pointer.connection
    grab = lambda: ImageGrab.grab(xdisplay=strata.display.display)
    try:
        strata.pointer.move_to(*target)
        strata.settle(source)
        resting_y, resting_contrast = text_position(grab(), bounds)
        strata.pointer.drag_points(strata.pointer.drag_origin(source), target, release=False)
        if outcome == "copy":
            connection.key(MODIFIER_KEYSYMS["ctrl"], True)
            strata.pointer.move_to(*target)
        _, dragging_contrast = text_position(grab(), bounds)
        assert dragging_contrast < resting_contrast * 0.8, "a real source drag must start"

        released = time.monotonic()
        if outcome == "escape":
            connection.key(keysym("Escape"), True)
            connection.key(keysym("Escape"), False)
        connection.button(1, False)
        samples = []
        # Sample at exact deadlines, not the UI readiness poller's 50 ms intervals.
        for delay in (0.06, 0.12, 0.18):
            time.sleep(max(0, released + delay - time.monotonic()))
            image = grab()
            samples.append((time.monotonic() - released, image))
        samples = [(elapsed, text_position(image, bounds)) for elapsed, image in samples]
        assert all(elapsed < 0.24 for elapsed, _ in samples), samples
        for _, (position, _) in samples:
            assert abs(position - resting_y) < 1.5, "source label slid out after drag completion"
        assert samples[-1][1][1] > dragging_contrast * 1.25, "dragging opacity must recover"

        if outcome in ("copy", "move"):
            strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "the transferred file")
            assert fixture.path("archive/todo.txt").read_bytes() == original
            strata.entry("todo.txt", directory="archive")
            if outcome == "move":
                strata.wait(lambda: not fixture.path("todo.txt").exists(), "source removal")
                strata.wait_for_entry_gone("todo.txt", directory=fixture.root.name)
            else:
                assert fixture.path("todo.txt").read_bytes() == original
        else:
            if outcome == "failed":
                strata.wait(lambda: "could not" in strata.diagnostics().lower()
                            or "permission denied" in strata.diagnostics().lower(), "transfer failure")
            strata.wait(lambda: time.monotonic() - released >= 0.6,
                        "the deferred drop handler to finish")
            strata.entry("todo.txt")
            assert fixture.path("todo.txt").read_bytes() == original
            assert fixture.names("archive") == []
    finally:
        connection.button(1, False)
        connection.key(MODIFIER_KEYSYMS["ctrl"], False)
        fixture.path("archive").chmod(0o755)
