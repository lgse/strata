# SPDX-License-Identifier: MIT
"""Creating, renaming, trashing, deleting, and undoing."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES


def start_new_file(strata, select=True):
    if select:
        strata.select_entry("readme.md")
    strata.pointer.right_click(strata.pane(), at=strata.background_point())
    strata.choose_menu_item("New File")
    field = strata.editable_field()
    strata.wait(lambda: field.text.startswith("new file"), "the created file's editor")
    return field


@pytest.mark.parametrize("mode", ALL_MODES)
def test_invalid_new_file_names_can_be_corrected(strata, mode):
    name = "bad/name"
    field = start_new_file(strata)
    original = strata.fixture.names()
    strata.keyboard.type_text(name)
    strata.wait(lambda: field.text == name, "the invalid name to appear")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.window.find(role="text", name="Rename", states={"editable"}) is None, "the invalid edit to close")
    assert strata.fixture.names() == original
    strata.select_entry_with_keyboard("new file")
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("corrected")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.fixture.path("corrected").is_file(), "the corrected file on disk")
    strata.entry("corrected")


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", ["file", "folder"])
def test_clicking_inside_keeps_the_new_entry_and_preserves_its_name(strata, mode, kind):
    name = " padded "
    if kind == "folder":
        strata.select_entry("readme.md")
        strata.keyboard.press("ctrl+shift+n")
        field = strata.editable_field()
    else:
        field = start_new_file(strata)
    strata.keyboard.type_text(name)
    strata.wait(lambda: field.text == name, "the name to appear")
    strata.pointer.click(field)
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.fixture.path(name).exists(), "the exact name on disk")
    assert strata.fixture.path(name).is_dir() == (kind == "folder")
    strata.wait(
        lambda: strata.window.find(role="text", states={"editable"}) is None,
        "the submitted prompt to close",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind,name", [("file", "todo.txt"), ("folder", "archive")])
def test_creating_an_existing_name_does_not_overwrite(strata, mode, kind, name):
    strata.select_entry("readme.md")
    if kind == "folder":
        strata.keyboard.press("ctrl+shift+n")
    else:
        strata.pointer.right_click(strata.pane(), at=strata.background_point())
        strata.choose_menu_item("New File")
    field = strata.editable_field()
    original = strata.fixture.listing()
    strata.keyboard.type_text(name)
    strata.wait(lambda: field.text == name, "the existing name to appear")
    strata.keyboard.press("Return")
    dialog = strata.wait_for_dialog()
    assert dialog.name == "Unable to rename item"
    strata.pointer.click(strata.dialog_button("Close"))
    strata.wait(lambda: strata.dialog() is None, "the error to be dismissible")
    assert strata.fixture.listing() == original
    assert strata.fixture.path("todo.txt").read_text() == "todo\n"
    root = strata.fixture.root.name
    strata.select_entry("readme.md", root)
    strata.wait_for_selection(["readme.md"], root)
    if mode == "Columns" and kind == "folder":
        assert strata.pane_names() == [root, "new folder"]


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", ["file", "folder"])
def test_new_items_can_be_created_in_an_initially_empty_directory(strata, mode, kind):
    strata.open_directory("archive")
    if kind == "folder":
        strata.keyboard.press("ctrl+shift+n")
        strata.editable_field()
    else:
        start_new_file(strata, select=False)
    strata.keyboard.type_text("discarded")
    strata.keyboard.press("Escape")
    strata.entry("new " + kind, directory="archive")
    if kind == "folder":
        strata.keyboard.press("ctrl+shift+n")
        field = strata.editable_field()
    else:
        field = start_new_file(strata, select=False)
    strata.keyboard.type_text("kept")
    strata.wait(lambda: field.text == "kept", "the replacement name to appear")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.fixture.path("archive/kept").exists(), "the renamed item on disk")
    assert strata.fixture.path("archive/kept").is_dir() == (kind == "folder")
    strata.entry("kept", directory="archive")
    assert not strata.fixture.path("archive/discarded").exists()
    assert "Gtk-CRITICAL" not in strata.application.log()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("shortcut", ["F2", "ctrl+r"])
def test_rename_shortcuts(strata, mode, shortcut):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press(shortcut)
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("renamed.txt")
    strata.wait(lambda: field.text == "renamed.txt", "the new name to be typed")
    strata.keyboard.press("ctrl+r")
    assert strata.editable_field().text == "renamed.txt"
    strata.keyboard.press("Return")

    strata.wait(
        lambda: fixture.path("renamed.txt").exists(),
        "the file to be renamed on disk",
    )
    assert not fixture.path("todo.txt").exists()
    assert fixture.path("renamed.txt").read_text() == "todo\n"
    strata.entry("renamed.txt")


@pytest.mark.parametrize("shortcut", ["F2", "ctrl+r"])
def test_rename_shortcuts_leave_location_editing_alone(strata, shortcut):
    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+l")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(strata.fixture.path("documents")))
    strata.wait(lambda: field.text.endswith("documents"), "the location to be typed")
    strata.keyboard.press(shortcut)
    assert strata.editable_field().text == str(strata.fixture.path("documents"))
    strata.keyboard.press("Return")
    strata.wait_for_directory("documents")
    assert strata.fixture.path("todo.txt").exists()


def test_rename_from_the_context_menu(strata):
    fixture = strata.fixture

    strata.open_context_menu("readme.md")
    strata.choose_menu_item("Rename")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("guide.md")
    strata.wait(lambda: field.text == "guide.md", "the new name to be typed")
    strata.keyboard.press("Return")

    strata.wait(lambda: fixture.path("guide.md").exists(), "the rename to apply")
    assert not fixture.path("readme.md").exists()


def test_delete_moves_the_entry_to_trash(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("Delete")

    strata.wait(
        lambda: not fixture.path("todo.txt").exists(),
        "the file to leave the fixture tree",
    )
    strata.wait_for_entry_gone("todo.txt")
    trashed = strata.environment.trash_files
    assert trashed.exists() and any(trashed.iterdir()), (
        "the file should be recoverable from the isolated trash directory"
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_permanent_delete_requires_confirmation_and_can_be_cancelled(strata, mode):
    fixture = strata.fixture
    assert strata.view_mode() == mode

    strata.select_entry("todo.txt")
    strata.keyboard.press("shift+Delete")

    dialog = strata.wait_for_dialog()
    assert dialog.role in ("dialog", "alert")
    assert dialog.name == "Permanently delete 1 item?", (
        f"unexpected dialog {dialog.name!r}"
    )
    assert fixture.path("todo.txt").exists(), (
        "nothing may be deleted before the confirmation is answered"
    )

    strata.pointer.click(strata.dialog_button("Cancel"))
    strata.wait(lambda: strata.dialog() is None, "the dialog to close")
    assert fixture.path("todo.txt").exists(), "cancelling must keep the file"

    strata.select_entry("todo.txt")
    strata.keyboard.press("shift+Delete")
    strata.wait_for_dialog()
    strata.pointer.click(strata.dialog_button("Permanently delete 1 item"))

    strata.wait(
        lambda: not fixture.path("todo.txt").exists(),
        "the file to be deleted",
    )
    strata.wait_for_entry_gone("todo.txt")
    trashed = strata.environment.trash_files
    assert not trashed.exists() or not any(trashed.iterdir()), (
        "a permanent delete must not go through the trash"
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_permanent_delete_through_a_symlinked_parent(strata, mode):
    fixture = strata.fixture
    alias = fixture.path("documents-alias")
    alias.symlink_to(fixture.path("documents"), target_is_directory=True)

    strata.open_directory("documents-alias")
    strata.wait_for_directory("documents-alias")

    strata.select_entry("notes.txt", directory="documents-alias")
    strata.keyboard.press("shift+Delete")
    strata.wait_for_dialog()
    strata.pointer.click(strata.dialog_button("Permanently delete 1 item"))

    strata.wait(
        lambda: not fixture.path("documents/notes.txt").exists()
        or (strata.dialog() is not None and strata.dialog().name == "Completed with errors"),
        "the delete operation to finish",
    )
    assert not fixture.path("documents/notes.txt").exists()
    strata.wait_for_entry_gone("notes.txt", directory="documents-alias")
    strata.wait(lambda: strata.dialog() is None, "the confirmation dialog to close")
    assert alias.is_symlink()
    assert fixture.path("documents").is_dir()


def test_undo_restores_a_completed_move(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+x")
    strata.open_directory("archive")
    strata.paste_into("archive")
    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the move to complete",
    )

    strata.keyboard.press("ctrl+z")

    strata.wait(
        lambda: fixture.path("todo.txt").exists(),
        "undo to put the file back",
    )
    assert not fixture.path("archive/todo.txt").exists()
