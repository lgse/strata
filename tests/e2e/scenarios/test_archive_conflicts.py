# SPDX-License-Identifier: MIT

import tarfile
import zipfile

import pytest

from harness.modes import ALL_MODES


@pytest.fixture
def fixture_tree(fixture_tree, request):
    extension = request.node.callspec.params.get("extension", "zip")
    fixture_tree.path(f"archive.{extension}").write_bytes(b"original archive")
    fixture_tree.path(f"archive (1).{extension}").write_bytes(b"previous archive")
    return fixture_tree


def request_archive_collision(strata, format="ZIP"):
    strata.select_entry("todo.txt")
    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Compress…")
    dialog = strata.wait_for_dialog()
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("archive")
    strata.wait(lambda: field.text == "archive", "archive name input")
    strata.pointer.click(dialog.find(role="toggle button", name=format))
    strata.pointer.click(strata.dialog_button("Compress"))
    strata.wait(
        lambda: (dialog := strata.dialog()) is not None and dialog.name == "File already exists",
        "archive collision prompt",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("format,extension", [("ZIP", "zip"), ("TAR.GZ", "tar.gz")])
@pytest.mark.parametrize("activation", ["pointer", "Return"])
def test_keep_both_preserves_archives_and_selects_each_numbered_output(strata, mode, format, extension, activation):
    fixture = strata.fixture
    original = fixture.path(f"archive.{extension}")
    previous = fixture.path(f"archive (1).{extension}")
    for suffix in [2, 3]:
        request_archive_collision(strata, format)
        if activation == "pointer":
            strata.pointer.click(strata.dialog_button("Keep Both"))
        else:
            strata.wait(lambda: strata.dialog_button("Replace").has_state("focused"), "initial Replace focus")
            strata.keyboard.press("shift+Tab")
            strata.wait(lambda: strata.dialog_button("Keep Both").has_state("focused"), "Keep Both focus")
            strata.keyboard.press("Return")
        name = f"archive ({suffix}).{extension}"
        path = fixture.path(name)
        strata.wait(path.exists, "numbered archive publication")
        strata.wait(lambda: strata.dialog() is None, "archive progress dismissal")
        strata.wait_for_selection([name])
        strata.wait(lambda: strata.on_screen(strata.entry(name)), "numbered archive reveal")
        if format == "ZIP":
            with zipfile.ZipFile(path) as archive:
                assert archive.namelist() == ["todo.txt"]
                assert archive.read("todo.txt") == fixture.path("todo.txt").read_bytes()
        else:
            with tarfile.open(path, "r:gz") as archive:
                assert archive.getnames() == ["todo.txt"]
                assert archive.extractfile("todo.txt").read() == fixture.path("todo.txt").read_bytes()
        assert original.read_bytes() == b"original archive"
        assert previous.read_bytes() == b"previous archive"


@pytest.mark.parametrize("extension", ["zip"])
def test_undoing_a_compression_trashes_the_numbered_archive(strata, extension):
    fixture = strata.fixture
    trashed = strata.environment.trash_files

    request_archive_collision(strata)
    strata.pointer.click(strata.dialog_button("Keep Both"))
    numbered = fixture.path("archive (2).zip")
    strata.wait(numbered.exists, "numbered archive publication")
    strata.wait(lambda: strata.dialog() is None, "archive progress dismissal")

    strata.keyboard.press("ctrl+z")

    strata.wait(lambda: not numbered.exists(), "compress undo to trash the archive")
    strata.wait(lambda: any(trashed.iterdir()), "the archive to land in Trash")
    assert fixture.path("archive.zip").read_bytes() == b"original archive"
    assert fixture.path("archive (1).zip").read_bytes() == b"previous archive"


@pytest.mark.parametrize("choice", ["Cancel", "Escape", "Replace"])
def test_archive_conflict_keyboard_choices_preserve_cancel_and_replace_behavior(strata, choice):
    original = strata.fixture.path("archive.zip")
    request_archive_collision(strata)
    strata.wait(lambda: strata.dialog_button("Replace").has_state("focused"), "initial Replace focus")
    if choice == "Escape":
        strata.keyboard.press("Escape")
    else:
        if choice == "Cancel":
            strata.keyboard.press("shift+Tab")
            strata.keyboard.press("shift+Tab")
            strata.wait(lambda: strata.dialog_button("Cancel").has_state("focused"), "Cancel focus")
        strata.keyboard.press("Return")
    strata.wait(lambda: strata.dialog() is None, "conflict dismissal")
    assert strata.fixture.path("archive (1).zip").read_bytes() == b"previous archive"
    assert not strata.fixture.path("archive (2).zip").exists()
    if choice == "Replace":
        with zipfile.ZipFile(original) as archive:
            assert archive.namelist() == ["todo.txt"]
            assert archive.read("todo.txt") == strata.fixture.path("todo.txt").read_bytes()
    else:
        assert original.read_bytes() == b"original archive"
