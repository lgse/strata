# SPDX-License-Identifier: MIT

import pytest


@pytest.mark.preferences(browser_mode="columns")
@pytest.mark.parametrize("name", ["short", "portable-device-with-a-very-long-directory-name-" * 4])
def test_column_header_titles_stay_bounded_and_centered(strata, name):
    strata.fixture.path(name).mkdir()
    strata.entry(name)
    strata.open_directory(name)
    pane = strata.pane(name)
    strata.pointer.move_to(*pane.screen_bounds().center)
    title = strata.wait(lambda: pane.find(role="label", name=name), "the column title")
    refresh = strata.wait(
        lambda: pane.find(role="button", name="Refresh (F5)"),
        "the column actions",
    )
    strata.wait(lambda: refresh.screen_bounds().height > 0, "allocated actions")
    bounds = pane.screen_bounds()
    heading = title.screen_bounds()
    button = refresh.screen_bounds()
    header = title.parent.parent.screen_bounds()
    assert bounds.width <= 320
    assert heading.x + heading.width <= button.x
    assert abs(heading.center[1] - button.center[1]) <= 2
    actions = refresh.parent.screen_bounds()
    above = actions.y - bounds.y
    below = bounds.y + header.height - actions.y - actions.height
    assert abs(above - below) <= 1
