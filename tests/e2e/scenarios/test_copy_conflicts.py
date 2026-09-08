# SPDX-License-Identifier: GPL-3.0-or-later

import pytest

from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("activation", ["pointer", "Return"])
def test_keep_both_selects_the_numbered_copy_and_undo_preserves_originals(strata, mode, activation):
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
    strata.keyboard.press("ctrl+z")
    strata.wait(lambda: not fixture.path("archive/todo (2).txt").exists(), "copy undo")
    assert fixture.path("todo.txt").read_text() == "todo\n"
    assert fixture.path("archive/todo.txt").read_text() == "existing\n"
    assert fixture.path("archive/todo (1).txt").read_text() == "previous copy\n"


@pytest.mark.parametrize("moving", [False, True])
@pytest.mark.parametrize("action", ["Cancel", "Escape", "Close dialog"])
def test_dismissing_a_copy_or_move_conflict_preserves_both_files(strata, moving, action):
    fixture = strata.fixture
    fixture.path("archive/todo.txt").write_text("existing\n")
    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+x" if moving else "ctrl+c")
    strata.open_directory("archive")
    strata.paste_into("archive")
    dialog = strata.wait_for_dialog()
    assert (dialog.find(role="button", name="Keep Both") is not None) == (not moving)

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
    apply_all = dialog.find(name="Apply this choice to all remaining conflicts")
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
