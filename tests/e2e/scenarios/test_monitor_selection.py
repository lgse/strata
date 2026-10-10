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
def test_background_rename_preserves_scroll_and_multiselection(strata, mode):
    strata.select_entry("001.txt")
    strata.pointer.click(strata.entry("003.txt"), modifiers=["ctrl"])
    strata.wait_for_selection(["001.txt", "003.txt"])
    before = visible_entries(strata)[0].name
    strata.pointer.scroll(at=strata.pane().screen_bounds().center, clicks=2)
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


@pytest.mark.parametrize("mode", ALL_MODES)
def test_keyboard_navigation_survives_a_rescan_burst(strata, mode):
    strata.select_entry("003.txt")
    strata.wait_for_focused_entry("003.txt")
    # More changes than the monitor queues one by one (4,096), so the folder is
    # rescanned rather than spliced.
    for index in range(4200):
        strata.fixture.path(f"zz-{index:04}.txt").write_text("burst\n")
    strata.wait(
        lambda: strata.window.find(
            role="label",
            predicate=lambda node: node.description.startswith("1 of 4,300 items selected"),
        ),
        "the status bar to count the whole burst with the selection kept",
    )
    strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    strata.wait_for_focused_entry("004.txt")
    strata.wait_for_selection(["004.txt"])
