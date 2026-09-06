# SPDX-License-Identifier: GPL-3.0-or-later
"""Single-click and double-click activation, and switching between them."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES

SINGLE_CLICK = pytest.mark.preferences(
    list_folder_clicks=1, list_file_clicks=1,
    grid_folder_clicks=1, grid_file_clicks=1,
    explorer_folder_clicks=1, explorer_file_clicks=1,
)
DOUBLE_CLICK = pytest.mark.preferences(
    list_folder_clicks=2, list_file_clicks=2,
    grid_folder_clicks=2, grid_file_clicks=2,
    explorer_folder_clicks=2, explorer_file_clicks=2,
)


@SINGLE_CLICK
@pytest.mark.parametrize("mode", ALL_MODES)
def test_single_click_opens_a_directory(strata, mode):
    strata.pointer.click(strata.entry("documents"))

    strata.wait(
        lambda: strata.pane().name == "documents",
        "one click to open the directory in single-click mode",
    )
    strata.entry("notes.txt")


@SINGLE_CLICK
@pytest.mark.parametrize("mode", ALL_MODES)
def test_keyboard_selection_still_works_in_single_click_mode(strata, mode):
    strata.keyboard.press("Down")
    strata.wait(lambda: strata.focused_name() is not None, "keyboard focus")
    focused = strata.focused_name()

    strata.wait(
        lambda: strata.selected_names() == [focused],
        "the keyboard to select without opening anything",
    )
    assert strata.pane().name == strata.fixture.root.name, (
        "moving the keyboard cursor must not navigate in single-click mode"
    )


@DOUBLE_CLICK
@pytest.mark.parametrize("mode", ALL_MODES)
def test_one_click_only_selects_in_double_click_mode(strata, mode):
    root = strata.fixture.root.name

    strata.pointer.click(strata.entry("documents"))

    strata.wait(
        lambda: strata.selected_names() == ["documents"],
        "one click to select in double-click mode",
    )
    assert strata.pane().name == root, "one click must not open the directory"


@DOUBLE_CLICK
@pytest.mark.parametrize("mode", ALL_MODES)
def test_two_clicks_open_in_double_click_mode(strata, mode):
    strata.pointer.double_click(strata.entry("documents"))

    strata.wait(
        lambda: strata.pane().name == "documents",
        "two clicks to open the directory",
    )


@DOUBLE_CLICK
@pytest.mark.parametrize("mode", ALL_MODES)
def test_two_slow_clicks_do_not_open(strata, mode):
    """Two clicks outside the double-click interval are two single clicks."""

    root = strata.fixture.root.name

    strata.pointer.click_twice_slowly(strata.entry("documents"))

    strata.wait(
        lambda: strata.selected_names() == ["documents"],
        "the entry to stay selected",
    )
    assert strata.pane().name == root


@DOUBLE_CLICK
@pytest.mark.preferences(browser_mode="list")
def test_changing_the_preference_takes_effect_without_restarting(strata):
    root = strata.fixture.root.name
    strata.pointer.click(strata.entry("documents"))
    strata.wait(
        lambda: strata.selected_names() == ["documents"],
        "double-click mode to only select",
    )
    assert strata.pane().name == root

    _choose_single_click(strata)

    strata.pointer.click(strata.entry("pictures"))
    strata.wait(
        lambda: strata.pane().name == "pictures",
        "the new preference to apply to the running window",
    )


def _choose_single_click(strata) -> None:
    """Switch the List view to single-click activation through Settings."""

    strata.pointer.click(strata.header_button("Settings"))
    option = strata.wait(
        lambda: strata.window.find(
            role="toggle button", name="List Folders 1 click", rendered=False
        ),
        "the List single-click option in Settings",
    )
    assert option.activate(), "the option should expose an accessible action"
    strata.wait(
        lambda: strata.environment.read_preferences().get("explorer_folder_clicks")
        == "1",
        "the single-click choice to be saved",
    )
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(
            role="toggle button", name="List Folders 1 click"
        )
        is None,
        "Settings to close",
    )
