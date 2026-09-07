# SPDX-License-Identifier: GPL-3.0-or-later
"""Escape dismisses transient UI before clearing the active pane selection."""

import pytest

from harness.modes import ALL_MODES, PREVIOUS_ENTRY_KEY


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("multiple", [False, True])
@pytest.mark.preferences(single_click_previews=False)
def test_escape_clears_selection_without_navigation(strata, mode, multiple):
    root = strata.fixture.root.name
    strata.select_entry("readme.md", root)
    if multiple:
        strata.click_entry_with("todo.txt", ["ctrl"], root)
    focused = "todo.txt" if multiple else "readme.md"
    strata.wait_for_selection(["readme.md", "todo.txt"] if multiple else [focused], root)
    panes = strata.pane_names()

    strata.keyboard.press("Escape")

    strata.wait_for_selection([], root)
    strata.wait_for_focused_entry(focused)
    assert strata.pane_names() == panes
    strata.keyboard.press(PREVIOUS_ENTRY_KEY[mode])
    strata.wait(lambda: bool(strata.selected_names(root)), "keyboard navigation to resume")


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_escape_only_clears_the_active_column(strata):
    root = strata.fixture.root.name
    strata.open_directory("documents", directory=root)
    strata.select_entry("notes.txt", "documents")
    parent_selection = strata.selected_names(root)
    strata.keyboard.press("Escape")
    strata.wait_for_selection([], "documents")
    assert strata.selected_names(root) == parent_selection
    assert strata.pane_names() == [root, "documents"]
    strata.wait_for_focused_entry("notes.txt")


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("surface", ["menu", "properties", "rename", "new-folder", "location", "filter", "preview"])
@pytest.mark.preferences(single_click_previews=False)
def test_escape_dismisses_transient_before_selection(strata, mode, surface):
    root = strata.fixture.root.name
    strata.select_entry("readme.md", root)
    if surface in ("menu", "properties"):
        strata.open_context_menu("readme.md", root)
        if surface == "properties":
            strata.choose_menu_item("Properties")
            strata.wait_for_dialog()
    elif surface == "preview":
        strata.keyboard.press("space")
        strata.wait(strata.preview, "preview to open")
    else:
        shortcut = {"rename": "F2", "new-folder": "ctrl+shift+n", "location": "ctrl+l", "filter": "ctrl+f"}[surface]
        strata.keyboard.press(shortcut)
        strata.editable_field()

    strata.keyboard.press("Escape")

    if surface == "menu":
        strata.wait_for_menu_closed()
    elif surface == "properties":
        strata.wait(lambda: strata.dialog() is None, "properties to close")
    elif surface == "preview":
        strata.wait(lambda: strata.preview() is None, "preview to close")
    strata.wait_for_selection(["readme.md"], root)
    strata.keyboard.press("Escape")
    strata.wait_for_selection([], root)
    assert strata.pane_names() == [root]
