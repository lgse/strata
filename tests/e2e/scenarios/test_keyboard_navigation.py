# SPDX-License-Identifier: GPL-3.0-or-later
"""Keyboard-only movement, activation, and multi-selection."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES, NEXT_ENTRY_KEY, PREVIOUS_ENTRY_KEY

ROOT_ENTRIES = ["archive", "documents", "pictures", "readme.md", "todo.txt"]


@pytest.mark.parametrize("mode", ALL_MODES)
def test_arrow_keys_move_focus_and_selection(strata, mode):
    assert strata.entry_names() == ROOT_ENTRIES

    # A file, so that a single click never navigates in any presentation.
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")

    strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    strata.wait_for_focused_entry("todo.txt")
    strata.wait(
        lambda: strata.selected_names() == ["todo.txt"],
        "the selection to follow focus",
    )

    strata.keyboard.press(PREVIOUS_ENTRY_KEY[mode])
    strata.wait_for_focused_entry("readme.md")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_enter_opens_the_focused_directory(strata, mode):
    strata.select_entry("readme.md")
    strata.keyboard.press("Home")
    strata.wait_for_focused_entry("archive")
    strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    strata.wait_for_focused_entry("documents")

    strata.keyboard.press("Return")

    strata.wait_for_directory("documents")
    strata.entry("notes.txt", directory="documents")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_alt_up_and_history_navigate_between_directories(strata, mode):
    root = strata.fixture.root.name

    strata.open_directory("documents")
    strata.keyboard.press("alt+Up")
    strata.wait_for_directory(root)

    strata.keyboard.press("alt+Left")
    strata.wait_for_directory("documents")

    strata.keyboard.press("alt+Right")
    strata.wait_for_directory(root)


@pytest.mark.parametrize("mode", ALL_MODES)
def test_shift_arrow_extends_the_selection(strata, mode):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")

    strata.keyboard.press(f"shift+{NEXT_ENTRY_KEY[mode]}")

    strata.wait(
        lambda: strata.selected_names() == ["readme.md", "todo.txt"],
        "shift and an arrow to extend the selection",
    )

    strata.keyboard.press(f"shift+{PREVIOUS_ENTRY_KEY[mode]}")
    strata.wait(
        lambda: strata.selected_names() == ["readme.md"],
        "shift and the opposite arrow to shrink the selection again",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_select_all_selects_every_entry(strata, mode):
    strata.select_entry("readme.md")

    strata.keyboard.press("ctrl+a")

    strata.wait(
        lambda: strata.selected_names() == ROOT_ENTRIES,
        "Ctrl+A to select every entry in the pane",
    )


def test_focus_stays_usable_after_changing_views(strata):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")

    strata.keyboard.press("ctrl+3")
    strata.wait_for_view("List")

    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("Down")
    strata.wait_for_focused_entry("todo.txt")

    strata.keyboard.press("ctrl+1")
    strata.wait_for_view("Columns")
    strata.keyboard.press("Up")
    strata.wait_for_focused_entry("readme.md")


def test_keyboard_only_copy_and_paste_round_trip(strata):
    """A complete file operation without ever touching the pointer."""

    fixture = strata.fixture
    strata.keyboard.press("Down")
    strata.wait(lambda: strata.focused_name() is not None, "initial keyboard focus")

    strata.keyboard.press("End")
    strata.wait_for_focused_entry("todo.txt")
    strata.keyboard.press("ctrl+c")

    strata.keyboard.press("Home")
    strata.wait_for_focused_entry("archive")
    strata.keyboard.press("Return")
    strata.wait_for_directory("archive")
    strata.keyboard.press("ctrl+v")

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the keyboard-only paste to land",
    )
    assert fixture.path("todo.txt").exists(), "a copy must leave the source alone"
