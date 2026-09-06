# SPDX-License-Identifier: GPL-3.0-or-later
"""Accessibility semantics the rest of the suite — and screen readers — rely on."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES

ROOT_ENTRIES = ["archive", "documents", "pictures", "readme.md", "todo.txt"]
FOLDERS = {"archive", "documents", "pictures"}


@pytest.fixture
def root(strata) -> str:
    return strata.fixture.root.name


@pytest.mark.parametrize("mode", ALL_MODES)
def test_every_entry_is_named_and_described(strata, mode, root):
    entries = strata.entries(root)

    assert [node.name for node in entries] == ROOT_ENTRIES
    for node in entries:
        expected = "Folder" if node.name in FOLDERS else "File"
        assert node.description == expected, (
            f"{node.name} should be described as a {expected}"
        )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_entries_expose_selection_semantics(strata, mode, root):
    # GTK 4.14 omits SELECTABLE on unselected rows; exercise SELECTED transitions.
    for node in strata.entries(root):
        assert "focusable" in node.states, f"{node.name} should be focusable"

    strata.select_entry("todo.txt", directory=root)

    selected = strata.entry("todo.txt", directory=root)
    assert "selected" in selected.states
    others = [
        node for node in strata.entries(root) if node.name != "todo.txt"
    ]
    assert all("selected" not in node.states for node in others)


@pytest.mark.parametrize("mode", ALL_MODES)
def test_the_pane_names_its_directory_and_presentation(strata, mode, root):
    pane = strata.pane(root)

    assert pane.name == root
    assert pane.description == f"{mode} view"


@pytest.mark.parametrize("mode", ALL_MODES)
def test_the_entry_list_is_named_after_its_directory(strata, mode, root):
    container = strata.entry_container(root)

    assert container is not None
    assert container.name == root
    assert container.description == "Files"


def test_the_context_menu_uses_menu_semantics(strata):
    strata.open_context_menu("todo.txt")

    menu = strata.context_menu()
    assert menu is not None, "the context menu should have the menu role"
    items = menu.find_all(role="menu item")
    assert items, "menu entries should have the menu item role"
    assert all(node.name for node in items), "every menu item needs a name"
    strata.dismiss_menu()


def test_dialogs_are_announced_as_dialogs(strata):
    strata.select_entry("todo.txt")
    strata.keyboard.press("shift+Delete")

    dialog = strata.wait_for_dialog()
    assert dialog.role in ("dialog", "alert")
    assert dialog.name, "a dialog needs an accessible name"

    strata.pointer.click(strata.dialog_button("Cancel"))
    strata.wait(lambda: strata.dialog() is None, "the dialog to close")


def test_toolbar_controls_are_named(strata):
    for name in (
        "Search (Ctrl+K)",
        "Appearance",
        "Settings",
        "Close window",
        "Toggle sidebar (Ctrl+B)",
    ):
        assert strata.window.find(name=name) is not None, f"{name!r} is unnamed"


def test_inline_fields_are_named(strata):
    strata.select_entry("todo.txt")
    strata.keyboard.press("F2")
    field = strata.editable_field()
    assert field.name == "Rename"
    strata.keyboard.press("Escape")

    strata.keyboard.press("ctrl+shift+n")
    field = strata.editable_field()
    assert field.name == "New item name"
    strata.keyboard.press("Escape")


def test_focus_order_reaches_the_files_from_the_header(strata):
    """Tab from the window's first control eventually reaches the listing."""

    strata.keyboard.press("Tab")
    seen = []
    for _ in range(20):
        focused = strata.focused_node()
        if focused is None:
            strata.keyboard.press("Tab")
            continue
        seen.append(f"{focused.role}:{focused.name}")
        if focused.role in ("list", "table") or strata.focused_name() is not None:
            return
        strata.keyboard.press("Tab")
    raise AssertionError(f"Tab never reached the file listing; visited {seen}")


def test_a_whole_file_operation_is_possible_with_the_keyboard_alone(strata):
    fixture = strata.fixture

    strata.select_entry_with_keyboard("todo.txt")
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("keyboard-only.txt")
    strata.keyboard.press("Return")

    strata.wait(
        lambda: fixture.path("keyboard-only.txt").exists(),
        "the keyboard-only rename to apply",
    )
