# SPDX-License-Identifier: MIT
"""Keyboard-only movement, activation, and multi-selection."""

from __future__ import annotations

import pytest
from harness.modes import ALL_MODES, COLUMNS_AND_ONE, NEXT_ENTRY_KEY, PREVIOUS_ENTRY_KEY

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


@pytest.mark.preferences(arrow_navigation_scoped=True, type_to_search=False)
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("bindings", ["arrows", "hjkl"])
def test_arrow_scope_keeps_focus_in_files_and_toggles_live(strata, mode, bindings):
    up, left = ("Up", "Left") if bindings == "arrows" else ("k", "h")
    strata.select_entry("readme.md")
    strata.keyboard.press("Home")
    strata.wait_for_focused_entry("archive")
    for key in [up, left, up]:
        strata.keyboard.press(key)
        strata.wait_for_focused_entry("archive")

    strata.keyboard.press("ctrl+\\")
    strata.wait(
        lambda: strata.environment.read_preferences().get("arrow_navigation_scoped") == "false",
        "arrow scope disabled by shortcut",
    )
    strata.keyboard.press(up)
    strata.wait(lambda: strata.focused_name() is None, "Up leaves the file list")
    strata.keyboard.press("Down")
    strata.wait_for_focused_entry("archive")
    strata.keyboard.press("ctrl+\\")
    strata.wait(
        lambda: strata.environment.read_preferences().get("arrow_navigation_scoped") == "true",
        "arrow scope enabled by shortcut",
    )
    strata.keyboard.press(up)
    strata.wait_for_focused_entry("archive")


@pytest.mark.preferences(browser_mode="icons", type_to_search=False)
def test_tenxer_icons_stay_on_tiles_at_edges_in_search_and_peek(strata):
    """Home-row Icons motion, including an edge, an empty folder, search hits, and i."""

    strata.keyboard.press("ctrl+shift+m")
    strata.wait(
        lambda: strata.environment.read_preferences().get("tenxer_mode") == "true",
        "10xer mode to turn on",
    )
    root = strata.current_directory()
    strata.keyboard.press("Home")
    strata.wait_for_focused_entry("archive")
    for key in ("Left", "h", "KP_Left"):
        strata.keyboard.press(key)
        strata.wait_for_focused_entry("archive")
        assert strata.current_directory() == root

    empty = strata.fixture.path("empty-icons")
    empty.mkdir()
    strata.keyboard.press("F5")
    strata.select_entry("empty-icons")
    strata.keyboard.press("o")
    strata.wait_for_directory("empty-icons")
    for key in ("h", "j", "k", "l", "Left", "Down", "i"):
        strata.keyboard.press(key)
        assert strata.current_directory() == "empty-icons"
        assert strata.peek() is None
    strata.keyboard.press("BackSpace")
    strata.wait_for_directory(root)

    strata.select_entry("documents")
    strata.keyboard.press("i")
    strata.wait(lambda: strata.peek() is not None, "i to open the folder peek")
    assert strata.current_directory() == root
    assert strata.focused_name() == "documents"
    strata.keyboard.press("i")
    strata.wait(lambda: strata.peek() is None, "a second i to close the folder peek")
    strata.select_entry("readme.md")
    strata.keyboard.press("i")
    assert strata.peek() is None
    assert strata.current_directory() == root

    strata.keyboard.press("ctrl+shift+m")
    strata.wait(
        lambda: strata.environment.read_preferences().get("tenxer_mode") == "false",
        "10xer mode to turn off",
    )
    strata.keyboard.press("ctrl+f")
    strata.editable_field()
    strata.keyboard.type_text("txt")
    strata.wait(lambda: len(strata.matches()) >= 2, "icon search hits")
    strata.keyboard.press("Down")
    strata.wait(
        lambda: strata.focused_name() in strata.matches(),
        "Down to focus a search hit",
    )
    strata.keyboard.press("ctrl+shift+m")
    strata.wait(
        lambda: strata.environment.read_preferences().get("tenxer_mode") == "true",
        "10xer mode to turn on over search results",
    )
    before = strata.current_directory()
    shown = list(strata.matches())
    strata.keyboard.press("Down")
    strata.keyboard.press("j")
    assert strata.current_directory() == before
    assert strata.matches() == shown, "directional keys dismissed the search results"
    assert strata.focused_name() in shown
    assert strata.peek() is None


@pytest.mark.preferences(tenxer_mode=True, type_to_search=True)
def test_tenxer_keeps_location_edit_and_skips_the_filter_shortcut(strata):
    strata.select_entry("readme.md")
    names = strata.entry_names()

    strata.keyboard.press("ctrl+f")
    strata.wait(
        lambda: strata.window.find(role="text", states={"editable", "focused"}) is None,
        "Ctrl+F does not open a filter while 10xer is on",
    )
    assert strata.entry_names() == names
    assert strata.environment.read_preferences().get("tenxer_mode") == "true"

    strata.keyboard.press("ctrl+l")
    field = strata.editable_field()
    strata.keyboard.press("q")
    strata.wait(lambda: "q" in field.text.lower(), "q is typed into the location field")
    assert strata.environment.read_preferences().get("tenxer_mode") == "true"
    assert strata.entry_names() == names

@pytest.mark.parametrize("mode", COLUMNS_AND_ONE)
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


@pytest.mark.parametrize("mode", COLUMNS_AND_ONE)
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
