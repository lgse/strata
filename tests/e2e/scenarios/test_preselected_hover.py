# SPDX-License-Identifier: MIT
"""A load cursor remains hoverable without a click to reset selection."""

import pytest
from PIL import Image, ImageChops, ImageDraw

from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("arrival", ["startup", "keyboard-entry"])
def test_preselected_entry_hover_before_any_click(strata, mode, arrival, tmp_path):
    if arrival == "keyboard-entry":
        strata.select_entry_with_keyboard("documents")
        strata.keyboard.press("Return")
        strata.wait_for_directory("documents")
        strata.wait_for_selection(["notes.txt"], "documents")
    strata.wait(lambda: strata.selected_names(), "the initial load cursor")
    selected = strata.selected_names()
    assert len(selected) == 1
    entry = strata.entry(selected[0])
    bounds = entry.screen_bounds()
    strata.park_pointer()
    crop = (bounds.x + 3, bounds.y + 3,
            bounds.x + bounds.width - 3, bounds.y + bounds.height - 3)
    with Image.open(strata.screenshot(tmp_path / "before.png")) as image:
        before = image.convert("RGB").crop(crop)
    strata.pointer.move_to(*bounds.center)

    def visibly_hovered():
        with Image.open(strata.screenshot(tmp_path / "hover.png")) as image:
            after = image.convert("RGB").crop(crop)
        difference = ImageChops.difference(before, after)
        # Exclude the pointer itself, even on capture backends that include it.
        x, y = bounds.center
        ImageDraw.Draw(difference).rectangle(
            (x - crop[0] - 2, y - crop[1] - 2,
             x - crop[0] + 32, y - crop[1] + 32), fill="black"
        )
        return difference.getbbox() is not None

    strata.wait(visibly_hovered, "hover feedback on the preselected entry without clicking")
    assert strata.selected_names() == selected
