# SPDX-License-Identifier: GPL-3.0-or-later
"""Creating, renaming, trashing, deleting, and undoing."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
def test_create_folder_from_the_keyboard(strata, mode):
    fixture = strata.fixture

    strata.select_entry("readme.md")
    strata.keyboard.press("ctrl+shift+n")
    field = strata.editable_field()
    strata.keyboard.type_text("new-folder")
    strata.wait(lambda: field.text == "new-folder", "the typed name to appear")
    strata.keyboard.press("Return")

    strata.wait(
        lambda: fixture.path("new-folder").is_dir(),
        "the folder to be created on disk",
    )
    strata.entry("new-folder")


def test_creating_a_folder_can_be_cancelled(strata):
    fixture = strata.fixture

    strata.select_entry("readme.md")
    strata.keyboard.press("ctrl+shift+n")
    strata.editable_field()
    strata.keyboard.type_text("discarded")
    strata.keyboard.press("Escape")

    strata.wait(
        lambda: strata.window.find(role="text", states={"editable"}) is None,
        "the inline field to close",
    )
    assert not fixture.path("discarded").exists()
    assert sorted(fixture.names()) == [
        ".hidden.txt",
        "archive",
        "documents",
        "pictures",
        "readme.md",
        "todo.txt",
    ]


@pytest.mark.parametrize("mode", ALL_MODES)
def test_rename_with_f2(strata, mode):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("F2")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("renamed.txt")
    strata.wait(lambda: field.text == "renamed.txt", "the new name to be typed")
    strata.keyboard.press("Return")

    strata.wait(
        lambda: fixture.path("renamed.txt").exists(),
        "the file to be renamed on disk",
    )
    assert not fixture.path("todo.txt").exists()
    assert fixture.path("renamed.txt").read_text() == "todo\n"
    strata.entry("renamed.txt")


def test_rename_can_be_cancelled(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("never-applied.txt")
    strata.keyboard.press("Escape")

    strata.entry("todo.txt")
    assert fixture.path("todo.txt").exists()
    assert not fixture.path("never-applied.txt").exists()


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


def test_permanent_delete_asks_for_confirmation(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("shift+Delete")

    dialog = strata.wait_for_dialog()
    assert dialog.name == "Permanently delete 1 item?", (
        f"unexpected dialog {dialog.name!r}"
    )
    assert fixture.path("todo.txt").exists(), (
        "nothing may be deleted before the confirmation is answered"
    )

    strata.pointer.click(strata.dialog_button("Cancel"))
    strata.wait(lambda: strata.dialog() is None, "the dialog to close")
    assert fixture.path("todo.txt").exists(), "cancelling must keep the file"


def test_permanent_delete_removes_the_entry_when_confirmed(strata):
    fixture = strata.fixture

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
