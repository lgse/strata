# SPDX-License-Identifier: GPL-3.0-or-later
"""Copy, cut, and paste through both the keyboard and the context menu."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
def test_copy_leaves_the_source_in_place(strata, mode):
    assert strata.view_mode() == mode
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+c")
    strata.open_directory("archive")
    strata.paste_into("archive")

    strata.wait(
        lambda: (fixture.path("archive/todo.txt")).exists(),
        "the copy to land in archive",
    )
    assert fixture.path("todo.txt").exists(), "copying must not remove the source"
    assert fixture.path("archive/todo.txt").read_text() == "todo\n"
    strata.entry("todo.txt", directory="archive")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_paste_targets_a_single_selected_directory(strata, mode):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+c")
    strata.select_entry_with_keyboard("archive")
    strata.keyboard.press("ctrl+v")

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the copy to land in the selected folder",
    )
    assert fixture.path("todo.txt").exists()


def test_paste_follows_a_child_column_opened_by_pointer(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+c")
    strata.open_directory("archive")
    strata.keyboard.press("ctrl+v")

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the copy to land in the opened child column",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_cut_moves_only_after_paste(strata, mode):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+x")

    assert fixture.path("todo.txt").exists(), (
        "cut must not touch the filesystem before the paste"
    )

    strata.open_directory("archive")
    strata.paste_into("archive")

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the cut file to arrive in archive",
    )
    strata.wait(
        lambda: not fixture.path("todo.txt").exists(),
        "the cut file to leave its source directory",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_context_menu_copy_and_paste(strata, mode):
    fixture = strata.fixture

    strata.open_context_menu("readme.md")
    assert "Copy" in strata.menu_items()
    strata.choose_menu_item("Copy")

    strata.open_directory("archive")
    _paste_from_context_menu(strata)

    strata.wait(
        lambda: fixture.path("archive/readme.md").exists(),
        "the context-menu copy to land in archive",
    )
    assert fixture.path("readme.md").exists()


@pytest.mark.parametrize("mode", ALL_MODES)
def test_context_menu_cut_and_paste(strata, mode):
    fixture = strata.fixture

    strata.open_context_menu("readme.md")
    strata.choose_menu_item("Cut")
    strata.open_directory("archive")
    _paste_from_context_menu(strata)

    strata.wait(
        lambda: fixture.path("archive/readme.md").exists()
        and not fixture.path("readme.md").exists(),
        "the context-menu cut to complete",
    )


def _paste_from_context_menu(strata):
    strata.pointer.right_click(strata.pane("archive"), at=strata.background_point("archive"))
    strata.wait(strata.context_menu, "the destination context menu")
    strata.choose_menu_item("Paste")


def test_pasting_a_duplicate_name_asks_before_replacing(strata):
    fixture = strata.fixture
    fixture.path("archive/todo.txt").write_text("existing\n")

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+c")
    strata.open_directory("archive")
    strata.paste_into("archive")

    dialog = strata.wait_for_dialog()
    assert dialog.name == "File already exists", (
        "a duplicate name must be surfaced rather than silently resolved"
    )

    strata.pointer.click(strata.dialog_button("Skip"))
    strata.wait(lambda: strata.dialog() is None, "the conflict dialog to close")
    assert fixture.path("archive/todo.txt").read_text() == "existing\n", (
        "skipping must leave the existing file alone"
    )


def test_replacing_on_a_duplicate_name_overwrites(strata):
    fixture = strata.fixture
    fixture.path("archive/todo.txt").write_text("existing\n")

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+c")
    strata.open_directory("archive")
    strata.paste_into("archive")

    strata.wait_for_dialog()
    strata.pointer.click(strata.dialog_button("Replace"))

    strata.wait(
        lambda: fixture.path("archive/todo.txt").read_text() == "todo\n",
        "the replaced file to take the pasted contents",
    )
    assert fixture.path("todo.txt").exists(), "the copy source must survive"


def test_paste_availability_follows_the_clipboard(strata):
    def paste_is_enabled() -> bool:
        strata.pointer.right_click(strata.pane(), at=strata.background_point())
        strata.wait(strata.context_menu, "the pane context menu")
        enabled = strata.menu_item("Paste").has_state("sensitive")
        strata.dismiss_menu()
        return enabled

    assert not paste_is_enabled(), (
        "Paste should be offered but disabled while the clipboard is empty"
    )

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+c")

    assert paste_is_enabled(), "copying should enable Paste"


def test_copying_a_directory_copies_its_contents(strata):
    fixture = strata.fixture

    # Through the context menu: in Columns a plain click on a folder opens it.
    strata.open_context_menu("documents")
    strata.choose_menu_item("Copy")
    strata.open_directory("archive")
    strata.paste_into("archive")

    strata.wait(
        lambda: fixture.path("archive/documents/notes.txt").exists(),
        "the directory copy to include its contents",
    )
    assert sorted(fixture.names("archive/documents")) == [
        "notes.txt",
        "report.md",
        "spreadsheet.csv",
    ]
    assert fixture.path("documents/notes.txt").exists()
