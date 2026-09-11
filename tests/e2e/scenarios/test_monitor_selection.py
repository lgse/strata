# SPDX-License-Identifier: MIT

import pytest

from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES, NEXT_ENTRY_KEY


@pytest.fixture
def fixture_tree():
    fixture = FixtureTree.create(
        {f"{index:03}.txt": f"{index}\n" for index in range(100)}
    )
    try:
        yield fixture
    finally:
        fixture.cleanup()


def visible_entries(strata):
    viewport = next(
        node.screen_bounds()
        for node in strata.entry_container().ancestors()
        if node.role == "scroll pane"
    )
    return [
        node for node in strata.entries()
        if viewport.y <= node.screen_bounds().y < viewport.y + viewport.height
    ]


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("wheel_clicks", [2, 12])
def test_background_rename_preserves_scroll_and_multiselection(strata, mode, wheel_clicks):
    strata.select_entry("001.txt")
    strata.pointer.click(strata.entry("003.txt"), modifiers=["ctrl"])
    strata.wait_for_selection(["001.txt", "003.txt"])
    before = visible_entries(strata)[0].name
    strata.pointer.scroll(at=strata.pane().screen_bounds().center, clicks=wheel_clicks)
    strata.wait(
        lambda: visible_entries(strata) and visible_entries(strata)[0].name != before,
        "the listing to scroll",
    )
    visible = visible_entries(strata)
    marker = strata.settle(visible[len(visible) // 2])
    scrolled = marker.screen_bounds().y

    source_name = visible[-1].name
    source = strata.fixture.path(source_name)
    destination_name = source.stem + "-renamed.txt"
    source.write_text("updated before rename\n")
    source.rename(strata.fixture.path(destination_name))
    strata.entry(destination_name)
    strata.wait_for_entry_gone(source_name)
    strata.settle(marker)

    assert marker.screen_bounds().y == scrolled
    strata.pointer.scroll(at=strata.pane().screen_bounds().center, clicks=20, down=False)
    strata.wait_for_selection(["001.txt", "003.txt"])
    assert not source.exists()
    assert strata.fixture.path(destination_name).read_text() == "updated before rename\n"


@pytest.mark.parametrize("mode", ALL_MODES)
def test_keyboard_navigation_survives_background_insertion(strata, mode):
    strata.select_entry("003.txt")
    strata.wait_for_focused_entry("003.txt")
    strata.fixture.path("000-new.txt").write_text("new entry\n")
    strata.entry("000-new.txt")
    strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    strata.wait_for_focused_entry("004.txt")
    strata.wait_for_selection(["004.txt"])
