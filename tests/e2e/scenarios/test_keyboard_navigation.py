# SPDX-License-Identifier: MIT
"""Keyboard-only movement, activation, and multi-selection."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES, NEXT_ENTRY_KEY, PREVIOUS_ENTRY_KEY

ROOT_ENTRIES = ["archive", "documents", "pictures", "readme.md", "todo.txt"]


@pytest.mark.preferences(type_to_search=False)
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("bindings", ["arrows", "hjkl"])
def test_arrow_keys_move_focus_and_selection(strata, mode, bindings):
    assert strata.entry_names() == ROOT_ENTRIES

    # A file, so that a single click never navigates in any presentation.
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")

    aliases = {"Left": "h", "Down": "j", "Up": "k", "Right": "l"}
    next_key = NEXT_ENTRY_KEY[mode]
    previous_key = PREVIOUS_ENTRY_KEY[mode]
    if bindings == "hjkl":
        next_key, previous_key = aliases[next_key], aliases[previous_key]
    strata.keyboard.press(next_key)
    strata.wait_for_focused_entry("todo.txt")
    strata.wait(
        lambda: strata.selected_names() == ["todo.txt"],
        "the selection to follow focus",
    )

    strata.keyboard.press(previous_key)
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


@pytest.mark.preferences(browser_mode="list")
@pytest.mark.parametrize("return_key", ["alt+Left", "alt+Up"])
@pytest.mark.parametrize("enter_with", ["keyboard", "pointer"])
def test_list_return_restores_nested_scroll_selection_and_keyboard_cursor(
    strata, return_key, enter_with
):
    def populate(parent):
        for index in range(160):
            (parent / f"folder-{index:03}").mkdir()

    def scroll_and_enter(parent, clicks):
        container = strata.entry_container()
        viewport = next(
            node.screen_bounds()
            for node in container.ancestors()
            if node.role == "scroll pane"
        )
        strata.pointer.scroll(at=viewport.center, clicks=clicks)

        def middle_entry():
            visible = [
                node for node in strata.entries()
                if viewport.y < node.screen_bounds().y
                < viewport.y + viewport.height - node.screen_bounds().height
            ]
            if visible and visible[0].name != "folder-000":
                return visible[len(visible) // 2]
            return None

        marker = strata.wait(middle_entry, "a scrolled directory viewport")
        name = marker.name
        populate(parent / name)
        strata.select_entry(name)
        strata.wait_for_focused_entry(name)
        y = strata.settle(strata.entry(name)).screen_bounds().y
        if enter_with == "keyboard":
            strata.keyboard.press("Return")
        else:
            strata.open_directory(name)
        strata.wait_for_directory(name)
        return name, y

    parent = strata.fixture.path("archive")
    populate(parent)
    strata.open_directory("archive")
    first, first_y = scroll_and_enter(parent, 18)
    second, second_y = scroll_and_enter(parent / first, 10)

    for directory, name, y in [(first, second, second_y), ("archive", first, first_y)]:
        strata.keyboard.press(return_key)
        strata.wait_for_directory(directory)
        strata.wait_for_selection([name])
        strata.wait_for_focused_entry(name)
        restored = strata.settle(strata.entry(name))
        assert abs(restored.screen_bounds().y - y) <= 2, "restore the viewport, not just reveal the selection"
        next_name = f"folder-{int(name.removeprefix('folder-')) + 1:03}"
        strata.keyboard.press("Down")
        strata.wait_for_focused_entry(next_name)
        strata.wait_for_selection([next_name])


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
