# SPDX-License-Identifier: MIT
"""The floating shelf stages file references without moving sources."""


def _shelf(strata):
    return strata.wait(
        lambda: strata.application.application_node.find(role="frame", name="Strata Shelf"),
        "the floating shelf to appear",
    )


def _open_shelf(strata):
    strata.pointer.click(strata.window.find(role="button", name="Show floating shelf"))
    return _shelf(strata)


def test_collect_from_two_folders_and_copy_to_current_folder(strata):
    fixture = strata.fixture
    shelf = _open_shelf(strata)
    strata.pointer.drag(strata.entry("todo.txt"), shelf)
    strata.wait(lambda: shelf.find(role="label", name="Shelf (1)"), "first file on shelf")
    assert fixture.path("todo.txt").read_text() == "todo\n"

    strata.open_directory("documents")
    strata.pointer.drag(strata.entry("notes.txt", directory="documents"), shelf)
    strata.wait(lambda: shelf.find(role="label", name="Shelf (2)"), "second file on shelf")
    assert fixture.path("documents/notes.txt").exists()

    strata.pointer.click(strata.window.find(role="button", name="New tab"))
    strata.keyboard.press("ctrl+l")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(fixture.path("archive")))
    strata.keyboard.press("Return")
    strata.wait_for_directory("archive")
    strata.pointer.click(shelf.find(role="button", name="Copy here"))
    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists()
        and fixture.path("archive/notes.txt").exists(),
        "both collected files to copy into archive",
    )
    assert fixture.path("todo.txt").exists()
    assert fixture.path("documents/notes.txt").exists()
    assert shelf.find(role="label", name="Shelf (2)") is not None


def test_shelf_is_shared_across_tabs(strata):
    fixture = strata.fixture
    shelf = _open_shelf(strata)
    strata.select_entry("todo.txt")
    strata.pointer.click(shelf.find(role="button", name="Add selection"))
    strata.wait(lambda: shelf.find(role="label", name="Shelf (1)"), "selection staged")

    strata.pointer.click(strata.window.find(role="button", name="New tab"))
    strata.open_directory("archive")
    strata.pointer.click(shelf.find(role="button", name="Copy here"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "copy from the other tab")
    assert fixture.path("todo.txt").exists()


def test_drag_shelved_file_into_folder(strata):
    fixture = strata.fixture
    shelf = _open_shelf(strata)
    strata.pointer.drag(strata.entry("todo.txt"), shelf)
    remove = strata.wait(
        lambda: shelf.find(role="button", name="Remove todo.txt from shelf"),
        "staged item row",
    )
    row = next(node for node in remove.ancestors() if node.role == "panel")
    strata.pointer.drag(row, strata.entry("archive"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "shelved file to arrive")
    assert not fixture.path("todo.txt").exists()
    assert shelf.find(role="label", name="Shelf (1)") is not None


def test_add_selection_then_move_explicitly(strata):
    fixture = strata.fixture
    shelf = _open_shelf(strata)
    strata.select_entry("todo.txt")
    strata.pointer.click(shelf.find(role="button", name="Add selection"))
    strata.wait(lambda: shelf.find(role="label", name="Shelf (1)"), "selection staged")
    assert fixture.path("todo.txt").exists()

    strata.open_directory("archive")
    strata.pointer.click(shelf.find(role="button", name="Move here"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "shelved file to move")
    assert not fixture.path("todo.txt").exists()
    strata.pointer.click(shelf.find(role="button", name="Clear shelf"))
    strata.wait(lambda: shelf.find(role="label", name="Shelf (0)"), "shelf cleared")
    assert fixture.path("archive/todo.txt").exists()
