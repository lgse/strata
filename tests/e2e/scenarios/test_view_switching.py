# SPDX-License-Identifier: GPL-3.0-or-later
"""Switching between the Columns, Icons, and List presentations."""

from __future__ import annotations

import pytest

MODES = ["Columns", "Icons", "List"]
MODE_SHORTCUTS = {"Columns": "ctrl+1", "Icons": "ctrl+2", "List": "ctrl+3"}
STORED_MODES = {"Columns": '"columns"', "Icons": '"icons"', "List": '"list"'}


@pytest.mark.parametrize("mode", ["Icons", "List"])
def test_appearance_menu_switches_presentation(strata, mode):
    assert strata.view_mode() == "Columns"
    directory = strata.fixture.root.name

    strata.switch_view(mode)

    assert strata.view_mode() == mode
    assert strata.pane().name == directory
    assert "documents" in strata.entry_names()


@pytest.mark.parametrize("mode", MODES)
def test_shortcut_switches_presentation(strata, mode):
    strata.select_entry("todo.txt")

    strata.keyboard.press(MODE_SHORTCUTS[mode])

    strata.wait_for_view(mode)
    assert strata.pane_names()[0] == strata.fixture.root.name
    strata.wait(
        lambda: "todo.txt" in strata.all_selected_names(),
        "the selection to survive the switch",
    )


def test_switching_preserves_directory_selection_and_sort(strata):
    strata.select_entry("pictures")
    ascending = strata.entry_names("pictures")
    assert ascending == ["diagram.txt", "photo.txt"]

    reverse_sort = strata.window.find_all(
        role="button", name="Ascending — click to reverse"
    )[-1]
    strata.pointer.click(reverse_sort)
    strata.wait(
        lambda: strata.entry_names("pictures") == list(reversed(ascending)),
        "the open pane to sort descending",
    )
    strata.select_entry("diagram.txt", directory="pictures")

    strata.keyboard.press(MODE_SHORTCUTS["List"])
    strata.wait_for_view("List")

    assert strata.pane().name == "pictures", (
        "switching should land on the directory the Columns view had open"
    )
    assert strata.entry_names() == list(reversed(ascending)), (
        "the descending sort should survive the switch"
    )
    strata.wait(
        lambda: strata.selected_names() == ["diagram.txt"],
        "the selection to survive the switch",
    )
    strata.wait_for_focused_entry("diagram.txt")


def test_round_trip_through_every_view_returns_to_columns(strata):
    for mode in ["Icons", "List", "Columns"]:
        strata.keyboard.press(MODE_SHORTCUTS[mode])
        strata.wait_for_view(mode)
        assert "documents" in strata.entry_names()


def test_the_chosen_view_is_remembered(strata):
    strata.switch_view("List")

    strata.wait(
        lambda: strata.environment.read_preferences().get("browser_mode")
        == STORED_MODES["List"],
        "the chosen view to be written to settings.toml",
    )


@pytest.mark.parametrize("mode", MODES)
def test_appearance_menu_marks_a_view_chosen_by_shortcut(strata, mode):
    strata.keyboard.press(MODE_SHORTCUTS[mode])
    strata.wait_for_view(mode)

    strata.open_appearance_menu()

    for candidate in MODES:
        option = strata.window.find(role="button", name=candidate)
        assert option is not None
        assert _has_check_mark(option) == (candidate == mode)

    grouping = strata.window.find(role="button", name="Group by file type")
    assert grouping is not None
    assert ("sensitive" in grouping.states) == (mode == "List")


def _has_check_mark(option) -> bool:
    """A chosen appearance option shows a trailing check image."""

    images = option.find_all(role="image")
    # The leading image is the option's own icon; a visible second image is the
    # check mark.
    return len(images) > 1
