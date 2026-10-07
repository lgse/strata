# SPDX-License-Identifier: MIT
"""Session-only staging without filesystem effects before transfer."""


def _shelf(strata, count):
    return strata.wait(
        lambda: strata.window.find(role="toggle button", name=f"Shelf ({count})"),
        f"shelf to hold {count} files",
    )


def test_collect_from_two_folders_and_copy_to_current_folder(strata):
    fixture = strata.fixture
    strata.pointer.drag(strata.entry("todo.txt"), _shelf(strata, 0))
    _shelf(strata, 1)
    assert fixture.path("todo.txt").read_text() == "todo\n"

    strata.open_directory("documents")
    strata.pointer.drag(strata.entry("notes.txt", directory="documents"), _shelf(strata, 1))
    _shelf(strata, 2)
    assert fixture.path("documents/notes.txt").exists()

    strata.keyboard.press("ctrl+l")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(fixture.path("archive")))
    strata.keyboard.press("Return")
    strata.wait_for_directory("archive")
    strata.pointer.click(strata.window.find(role="button", name="Copy here"))
    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists()
        and fixture.path("archive/notes.txt").exists(),
        "both collected files to copy into archive",
    )
    assert fixture.path("todo.txt").exists()
    assert fixture.path("documents/notes.txt").exists()
    _shelf(strata, 2)


def test_shelf_is_shared_across_tabs(strata):
    fixture = strata.fixture
    strata.select_entry("todo.txt")
    strata.pointer.click(strata.window.find(role="button", name="Add selection"))
    _shelf(strata, 1)

    strata.keyboard.press("ctrl+t")
    _shelf(strata, 1)
    strata.open_directory("archive")
    shelf = _shelf(strata, 1)
    strata.pointer.click(shelf)
    strata.pointer.click(strata.window.find(role="button", name="Copy here"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "copy from the other tab")
    assert fixture.path("todo.txt").exists()


def test_drag_shelved_file_into_folder(strata):
    fixture = strata.fixture
    strata.pointer.drag(strata.entry("todo.txt"), _shelf(strata, 0))
    remove = strata.wait(
        lambda: strata.window.find(role="button", name="Remove todo.txt from shelf"),
        "staged item row",
    )
    row = next(node for node in remove.ancestors() if node.role == "panel")
    strata.pointer.drag(row, strata.entry("archive"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "shelved file to arrive")
    assert not fixture.path("todo.txt").exists()
    _shelf(strata, 1)


def test_add_selection_then_move_explicitly(strata):
    fixture = strata.fixture
    strata.select_entry("todo.txt")
    strata.pointer.click(strata.window.find(role="button", name="Add selection"))
    _shelf(strata, 1)
    assert fixture.path("todo.txt").exists()

    strata.open_directory("archive")
    strata.pointer.click(strata.window.find(role="button", name="Move here"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "shelved file to move")
    assert not fixture.path("todo.txt").exists()
    strata.pointer.click(strata.window.find(role="button", name="Clear shelf"))
    _shelf(strata, 0)
    assert fixture.path("archive/todo.txt").exists()
