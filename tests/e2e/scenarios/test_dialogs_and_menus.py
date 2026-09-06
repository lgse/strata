# SPDX-License-Identifier: GPL-3.0-or-later
"""Context menus, dialogs, Escape handling, and invalid operations."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES

ENTRY_MENU_ITEMS = {"Open", "Cut", "Copy", "Rename", "Move to Trash", "Properties"}


@pytest.mark.parametrize("mode", ALL_MODES)
def test_the_entry_context_menu_offers_the_file_actions(strata, mode):
    strata.open_context_menu("todo.txt")

    offered = set(strata.menu_items())
    assert ENTRY_MENU_ITEMS <= offered, (
        f"missing {sorted(ENTRY_MENU_ITEMS - offered)} from {sorted(offered)}"
    )
    strata.dismiss_menu()


def test_escape_closes_the_context_menu_without_acting(strata):
    before = strata.fixture.listing()
    strata.open_context_menu("todo.txt")

    strata.keyboard.press("Escape")

    strata.wait(lambda: strata.context_menu() is None, "the menu to close")
    assert strata.fixture.listing() == before


def test_menu_items_carry_their_shortcut_as_a_description(strata):
    strata.open_context_menu("todo.txt")

    copy = strata.menu_item("Copy")
    assert copy.description == "Ctrl+C", (
        "the accelerator belongs in the description, not the name"
    )
    strata.dismiss_menu()


def test_the_pane_context_menu_offers_directory_actions(strata):
    strata.pointer.right_click(strata.pane(), at=strata.background_point())
    strata.wait(strata.context_menu, "the pane context menu")

    offered = set(strata.menu_items())
    assert {"New Folder", "Select All", "Refresh"} <= offered, (
        f"unexpected pane menu {sorted(offered)}"
    )
    strata.dismiss_menu()


def test_properties_opens_and_closes(strata):
    strata.open_context_menu("readme.md")
    strata.choose_menu_item("Properties")

    dialog = strata.wait_for_dialog()
    assert "readme.md" in dialog.dump(), "the dialog should describe the file"

    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "Escape to close the dialog")


def test_renaming_to_an_invalid_name_is_rejected(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("bad/name.txt")
    strata.keyboard.press("Return")

    assert fixture.path("todo.txt").exists(), (
        "a name containing a separator must not be applied"
    )
    assert not (fixture.root / "bad").exists()
    strata.keyboard.press("Escape")


def test_renaming_onto_an_existing_name_is_rejected(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("readme.md")
    strata.keyboard.press("Return")

    assert fixture.path("todo.txt").exists(), "the rename must not silently succeed"
    assert fixture.path("readme.md").read_text() == "# Fixture\n", (
        "the existing file must keep its contents"
    )
    strata.keyboard.press("Escape")


def test_the_shortcut_reference_opens_and_closes(strata):
    strata.keyboard.press("F1")

    strata.wait(
        lambda: strata.window.find(role="label", name="Keyboard shortcuts"),
        "the shortcut reference to open",
    )

    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="label", name="Keyboard shortcuts") is None,
        "Escape to close the shortcut reference",
    )
