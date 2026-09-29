# SPDX-License-Identifier: MIT

import pytest


@pytest.mark.parametrize("activation", ["pointer", "Return"])
def test_keep_both_selects_the_numbered_copy_and_undo_preserves_originals(strata, activation):
    fixture = strata.fixture
    fixture.path("archive/todo.txt").write_text("existing\n")
    fixture.path("archive/todo (1).txt").write_text("previous copy\n")
    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+c")
    strata.open_directory("archive")
    strata.paste_into("archive")
    strata.wait_for_dialog()

    if activation == "pointer":
        strata.pointer.click(strata.dialog_button("Keep Both"))
    else:
        strata.wait(
            lambda: strata.dialog_button("Replace").has_state("focused"),
            "Replace to receive initial focus",
        )
        strata.keyboard.press("shift+Tab")
        strata.wait(
            lambda: strata.dialog_button("Keep Both").has_state("focused"),
            "Keep Both to receive keyboard focus",
        )
        strata.keyboard.press("Return")

    strata.wait(lambda: fixture.path("archive/todo (2).txt").exists(), "the numbered copy")
    strata.wait(
        lambda: strata.selected_names("archive") == ["todo (2).txt"],
        "the numbered copy to be selected",
    )
    assert fixture.path("archive/todo (2).txt").read_text() == "todo\n"
    # The copy can finish while the dismissing modal still owns keyboard input.
    strata.wait(lambda: strata.dialog() is None, "the conflict dialog to finish dismissing")
    strata.wait_for_focused_entry("todo (2).txt")
    strata.keyboard.press("ctrl+z")
    strata.wait(lambda: not fixture.path("archive/todo (2).txt").exists(), "copy undo")
    assert fixture.path("todo.txt").read_text() == "todo\n"
    assert fixture.path("archive/todo.txt").read_text() == "existing\n"
    assert fixture.path("archive/todo (1).txt").read_text() == "previous copy\n"


@pytest.mark.parametrize(
    "moving,action",
    [
        (False, "Cancel"),
        (False, "Escape"),
        (False, "Close dialog"),
        (True, "Cancel"),
    ],
    ids=["copy-Cancel", "copy-Escape", "copy-Close", "move-Cancel"],
)
def test_dismissing_a_copy_or_move_conflict_preserves_both_files(strata, moving, action):
    fixture = strata.fixture
    fixture.path("archive/todo.txt").write_text("existing\n")
    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+x" if moving else "ctrl+c")
    strata.open_directory("archive")
    strata.paste_into("archive")
    dialog = strata.wait_for_dialog()
    assert (dialog.find(role="button", name="Keep Both") is not None) == (not moving)
    assert dialog.find(role="button", name="Merge") is None, (
        "a file collision cannot merge"
    )

    if action == "Escape":
        strata.keyboard.press("Escape")
    else:
        strata.pointer.click(strata.dialog_button(action))
    strata.wait(lambda: strata.dialog() is None, "the conflict dialog to close")
    assert fixture.path("todo.txt").read_text() == "todo\n"
    assert fixture.path("archive/todo.txt").read_text() == "existing\n"
    assert not fixture.path("archive/todo (1).txt").exists()


def test_keep_both_applies_to_all_collisions_in_a_mixed_paste(strata):
    fixture = strata.fixture
    for name in ["notes.txt", "report.md"]:
        fixture.path(f"archive/{name}").write_text(f"existing {name}\n")
    strata.open_directory("documents")
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.press("ctrl+c")
    strata.keyboard.press("alt+Up")
    strata.wait_for_directory(fixture.root.name)
    strata.open_directory("archive")
    strata.paste_into("archive")
    dialog = strata.wait_for_dialog()
    apply_all = dialog.find(name="Apply to All")
    assert apply_all is not None, dialog.dump()
    strata.pointer.click(apply_all)
    strata.pointer.click(strata.dialog_button("Keep Both"))
    strata.wait(
        lambda: all(
            fixture.path(f"archive/{name}").exists()
            for name in ["notes (1).txt", "report (1).md", "spreadsheet.csv"]
        ),
        "all mixed-paste copies",
    )
    for name, copied in [
        ("notes.txt", "notes (1).txt"),
        ("report.md", "report (1).md"),
        ("spreadsheet.csv", "spreadsheet.csv"),
    ]:
        assert fixture.path(f"archive/{copied}").read_bytes() == fixture.path(
            f"documents/{name}"
        ).read_bytes()
    for name in ["notes.txt", "report.md"]:
        assert fixture.path(f"archive/{name}").read_text() == f"existing {name}\n"


def test_merge_combines_folder_contents(strata):
    fixture = strata.fixture
    fixture.path("folder").mkdir()
    fixture.path("folder/incoming.txt").write_text("from source\n")
    fixture.path("folder/shared.txt").write_text("incoming\n")
    fixture.path("archive/folder").mkdir()
    fixture.path("archive/folder/stays.txt").write_text("kept\n")
    fixture.path("archive/folder/shared.txt").write_text("original\n")
    strata.open_context_menu("folder")
    strata.choose_menu_item("Copy")
    strata.open_directory("archive")
    strata.paste_into("archive")

    strata.wait_for_dialog()
    strata.pointer.click(strata.dialog_button("Merge"))

    # The overwritten original is staged in Trash before the incoming copy
    # lands, so a bare existence check can catch the file mid-staging.
    strata.wait(
        lambda: fixture.path("archive/folder/incoming.txt").exists()
        and fixture.path("archive/folder/shared.txt").exists(),
        "the merged incoming files",
    )
    strata.wait(
        lambda: fixture.path("archive/folder/shared.txt").read_text() == "incoming\n",
        "the incoming item to overwrite a same-named destination item",
    )
    assert fixture.path("archive/folder/stays.txt").read_text() == "kept\n", (
        "destination-only contents survive the merge"
    )
    assert fixture.path("folder/incoming.txt").exists(), "a copy merge keeps the source"

    strata.wait(lambda: strata.dialog() is None, "the conflict dialog to finish dismissing")
    strata.keyboard.press("ctrl+z")
    strata.wait(
        lambda: not fixture.path("archive/folder/incoming.txt").exists(),
        "merge undo removes the created file",
    )
    assert fixture.path("archive/folder/shared.txt").read_text() == "original\n", (
        "merge undo restores the overwritten original from Trash"
    )
    assert fixture.path("archive/folder/stays.txt").read_text() == "kept\n", (
        "merge undo leaves destination-only contents alone"
    )
    assert fixture.path("archive/folder").is_dir(), "the destination folder survives undo"
    assert fixture.path("folder/incoming.txt").exists(), "the source is still intact"


def test_replacing_a_file_restores_the_original_on_undo(strata):
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
        "the replacement",
    )
    assert fixture.path("todo.txt").read_text() == "todo\n", "a copy keeps the source"

    strata.wait(lambda: strata.dialog() is None, "the conflict dialog to finish dismissing")
    strata.keyboard.press("ctrl+z")
    strata.wait(
        lambda: fixture.path("archive/todo.txt").read_text() == "existing\n",
        "replace undo restores the original staged in Trash",
    )
    assert fixture.path("todo.txt").read_text() == "todo\n"
