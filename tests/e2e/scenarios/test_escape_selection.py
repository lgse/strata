# SPDX-License-Identifier: MIT
"""Escape dismisses transient UI before clearing the active pane selection."""

import pytest

from harness.modes import ALL_MODES, NEXT_ENTRY_KEY

TRANSIENT_SURFACES = [
    "menu",
    "properties",
    "rename",
    "new-folder",
    "new-file",
    "location",
    "filter",
    "preview",
]
MODE_DEPENDENT_TRANSIENTS = {"new-folder", "preview", "rename"}


def _transient_dismiss_cases():
    cases = []
    for surface in TRANSIENT_SURFACES:
        modes = ALL_MODES if surface in MODE_DEPENDENT_TRANSIENTS else [
            mode for mode in ALL_MODES if mode.id == "columns"
        ]
        for mode in modes:
            cases.append(
                pytest.param(
                    mode.values[0],
                    surface,
                    marks=mode.marks,
                    id=f"{surface}-{mode.id}",
                )
            )
    return cases


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("multiple", [False, True])
@pytest.mark.preferences(single_click_previews=False)
def test_escape_clears_selection_without_navigation(strata, mode, multiple):
    root = strata.fixture.root.name
    if multiple:
        strata.select_entry("todo.txt", root)
        strata.click_entry_with("readme.md", ["ctrl"], root)
    else:
        strata.select_entry("readme.md", root)
    focused = "readme.md"
    strata.wait_for_selection(["readme.md", "todo.txt"] if multiple else [focused], root)
    panes = strata.pane_names()

    strata.keyboard.press("Escape")

    strata.wait_for_selection([], root)
    strata.wait_for_focused_entry(focused)
    assert strata.pane_names() == panes
    strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    strata.wait_for_selection(["todo.txt"], root)
    strata.wait_for_focused_entry("todo.txt")


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.preferences(single_click_previews=False)
def test_enter_after_escape_opens_the_focused_folder(strata, mode):
    root = strata.fixture.root.name
    strata.select_entry_with_keyboard("documents")
    strata.wait_for_focused_entry("documents")
    strata.keyboard.press("Escape")
    strata.wait_for_selection([], root)
    strata.wait_for_focused_entry("documents")

    strata.keyboard.press("Return")

    strata.wait_for_directory("documents")
    strata.wait_for_entries(["notes.txt", "report.md", "spreadsheet.csv"], "documents")


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


@pytest.mark.parametrize("mode,surface", _transient_dismiss_cases())
@pytest.mark.preferences(single_click_previews=False)
def test_escape_dismisses_transient_before_selection(strata, mode, surface):
    root = strata.fixture.root.name
    strata.select_entry("readme.md", root)
    if surface in ("menu", "properties"):
        strata.open_context_menu("readme.md", root)
        if surface == "properties":
            strata.choose_menu_item("Properties")
            strata.wait_for_dialog()
    elif surface == "new-file":
        strata.pointer.right_click(strata.pane(), at=strata.background_point())
        strata.choose_menu_item("New File")
        strata.editable_field()
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
    expected = {"new-folder": "new folder", "new-file": "new file"}.get(surface, "readme.md")
    strata.wait_for_selection([expected], root)
    if surface in ("new-folder", "new-file"):
        assert strata.fixture.path(expected).exists()
    strata.keyboard.press("Escape")
    strata.wait_for_selection([], root)
    expected_panes = [root, "new folder"] if mode == "Columns" and surface == "new-folder" else [root]
    assert strata.pane_names() == expected_panes


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("had_range", [False, True], ids=["no-range", "had-range"])
@pytest.mark.preferences(single_click_previews=False)
def test_shift_after_escape_starts_on_the_focused_entry(strata, mode, had_range):
    root = strata.fixture.root.name
    next_key = NEXT_ENTRY_KEY[mode]
    strata.wait_for_focused_entry("archive")
    strata.wait_for_selection(["archive"], root)
    if had_range:
        strata.keyboard.press(f"shift+{next_key}")
        strata.wait_for_selection(["archive", "documents"], root)
        strata.wait_for_focused_entry("documents")
        focused = "documents"
        first = ["documents"]
        second = ["documents", "pictures"]
    else:
        focused = "archive"
        first = ["archive"]
        second = ["archive", "documents"]

    strata.keyboard.press("Escape")
    strata.wait_for_selection([], root)
    strata.wait_for_focused_entry(focused)

    strata.keyboard.press(f"shift+{next_key}")
    strata.wait_for_selection(first, root)
    strata.wait_for_focused_entry(focused)

    strata.keyboard.press(f"shift+{next_key}")
    strata.wait_for_selection(second, root)
    strata.wait_for_focused_entry(second[-1])
