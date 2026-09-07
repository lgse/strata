# SPDX-License-Identifier: GPL-3.0-or-later
"""Context menus, dialogs, Escape handling, and invalid operations."""

from __future__ import annotations

import shutil

import pytest

from harness.modes import ALL_MODES

ENTRY_MENU_ITEMS = {"Open", "Cut", "Copy", "Rename", "Move to Trash", "Properties"}


@pytest.fixture
def executable_file(fixture_tree):
    program = fixture_tree.path("run-me")
    shutil.copy2(shutil.which("true"), program)
    return program


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


def _open_properties(strata, name):
    strata.open_context_menu(name)
    strata.choose_menu_item("Properties")
    return strata.wait_for_dialog()


def test_properties_opens_and_closes(strata):
    dialog = _open_properties(strata, "readme.md")
    assert "readme.md" in dialog.dump(), "the dialog should describe the file"

    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "Escape to close the dialog")


def test_executable_without_handler_requires_confirmation(executable_file, strata):
    strata.double_click_entry(executable_file.name)

    dialog = strata.wait_for_dialog()
    assert "Run this program?" in dialog.dump()
    strata.wait(
        lambda: "focused" in strata.dialog_button("Cancel").states,
        "Cancel to receive initial focus",
    )
    assert strata.dialog_button("Close dialog").activate()
    strata.wait(lambda: strata.dialog() is None, "the close button to dismiss the dialog")

    strata.double_click_entry(executable_file.name)
    strata.pointer.click(strata.dialog_button("Run"))
    strata.wait(lambda: strata.dialog() is None, "the confirmed program to launch")


def test_properties_pins_a_folder_and_offers_unpin_afterwards(strata):
    dialog = _open_properties(strata, "documents")
    pin = dialog.find(role="button", name="Pin")
    assert pin is not None, dialog.dump()
    assert "sensitive" in pin.states
    strata.pointer.click(pin)
    strata.wait(lambda: strata.dialog() is None, "the dialog to close after pinning")
    strata.wait(
        lambda: strata.window.find(role="button", name="documents"),
        "the pinned sidebar row",
    )

    dialog = _open_properties(strata, "documents")
    unpin = dialog.find(role="button", name="Unpin")
    assert unpin is not None, (
        f"Properties must offer Unpin for a pinned folder\n{dialog.dump()}"
    )
    assert "sensitive" in unpin.states, "the Unpin control must stay readable"
    assert dialog.find(role="button", name="Pin") is None

    strata.pointer.click(unpin)
    strata.wait(lambda: strata.dialog() is None, "the dialog to close after unpinning")
    strata.wait(
        lambda: strata.window.find(role="button", name="documents") is None,
        "the sidebar row to disappear",
    )


def test_properties_hides_the_pin_control_for_a_file(strata):
    dialog = _open_properties(strata, "readme.md")

    assert dialog.find(role="button", name="Pin") is None, dialog.dump()
    assert dialog.find(role="button", name="Unpin") is None, dialog.dump()

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


def compress_from_the_context_menu(strata, entry_name, archive_name):
    strata.open_context_menu(entry_name)
    strata.choose_menu_item("Compress…")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(archive_name)
    strata.wait(
        lambda: field.text == archive_name, f"{archive_name!r} to reach the name field"
    )
    return field


def test_enter_submits_the_compress_dialog(strata):
    compress_from_the_context_menu(strata, "readme.md", "bundle")

    strata.keyboard.press("Return")

    strata.wait(lambda: strata.dialog() is None, "the dialog to close")
    strata.wait(
        lambda: strata.fixture.path("bundle.zip").exists(),
        "Enter to create the archive",
    )


def test_an_invalid_archive_name_keeps_the_compress_dialog_open(strata):
    compress_from_the_context_menu(strata, "readme.md", "../escape")

    strata.keyboard.press("Return")

    dialog = strata.wait_for_dialog()
    assert dialog.name == "Compress 1 item", (
        f"an invalid name must keep the dialog open, got {dialog.name!r}"
    )
    assert not strata.fixture.path("escape.zip").exists(), (
        "an invalid name must not produce an archive"
    )
    strata.keyboard.press("Escape")


def test_enter_submits_the_extract_to_dialog(strata):
    compress_from_the_context_menu(strata, "readme.md", "bundle")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: strata.fixture.path("bundle.zip").exists(), "the archive to be created"
    )

    destination = strata.fixture.path("unpacked")
    strata.open_context_menu("bundle.zip")
    strata.choose_menu_item("Extract to…")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(destination))
    strata.wait(
        lambda: field.text == str(destination), "the destination to reach the field"
    )

    strata.keyboard.press("Return")

    strata.wait(
        lambda: (destination / "readme.md").exists(),
        "Enter to extract into the destination",
    )


def test_enter_submits_the_copy_to_dialog(strata):
    destination = strata.fixture.path("documents")

    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Copy to…")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(destination))
    strata.wait(
        lambda: field.text == str(destination), "the destination to reach the field"
    )

    strata.keyboard.press("Return")

    strata.wait(
        lambda: (destination / "todo.txt").exists(),
        "Enter to copy into the destination",
    )
