# SPDX-License-Identifier: MIT
"""The floating shelf stages file references without moving sources."""


def _shelf(strata):
    return strata.wait(
        lambda: strata.application.application_node.find(role="frame", name="Strata Shelf"),
        "the floating shelf to appear",
    )


def _open_shelf(strata):
    strata.keyboard.press("ctrl+shift+space")
    return _shelf(strata)


def _count(strata, count):
    return strata.wait(
        lambda: _shelf(strata).find(name=f"{count} item{'s' if count != 1 else ''}"),
        f"shelf to hold {count} items",
    )


def _action(strata, label):
    shelf = _shelf(strata)
    if shelf.find(name="Add selection", rendered=True) is None:
        strata.pointer.click(shelf.find(role="button", name="Shelf actions"))
    return strata.wait(
        lambda: shelf.find(name=label, rendered=True),
        f"shelf action {label} to appear",
    )


def test_collect_from_two_folders_and_copy_to_current_folder(strata):
    fixture = strata.fixture
    shelf = _open_shelf(strata)
    strata.pointer.drag(strata.entry("todo.txt"), shelf)
    _count(strata, 1)
    assert fixture.path("todo.txt").read_text() == "todo\n"

    strata.open_directory("documents")
    strata.pointer.drag(strata.entry("notes.txt", directory="documents"), shelf)
    _count(strata, 2)
    assert fixture.path("documents/notes.txt").exists()

    strata.pointer.click(strata.window.find(role="button", name="New tab"))
    strata.keyboard.press("ctrl+l")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(fixture.path("archive")))
    strata.keyboard.press("Return")
    strata.wait_for_directory("archive")
    strata.pointer.click(_action(strata, "Copy here"))
    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists()
        and fixture.path("archive/notes.txt").exists(),
        "both collected files to copy into archive",
    )
    assert fixture.path("todo.txt").exists()
    assert fixture.path("documents/notes.txt").exists()
    _count(strata, 2)


def test_shelf_is_shared_across_tabs(strata):
    fixture = strata.fixture
    _open_shelf(strata)
    strata.select_entry("todo.txt")
    strata.pointer.click(_action(strata, "Add selection"))
    _count(strata, 1)

    strata.pointer.click(strata.window.find(role="button", name="New tab"))
    strata.open_directory("archive")
    strata.pointer.click(_action(strata, "Copy here"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "copy from the other tab")
    assert fixture.path("todo.txt").exists()


def test_drag_shelved_stack_into_folder(strata):
    fixture = strata.fixture
    shelf = _open_shelf(strata)
    strata.pointer.drag(strata.entry("todo.txt"), shelf)
    _count(strata, 1)
    preview = shelf.find(role="panel", name="Shelf preview")
    assert preview is not None, shelf.dump()
    strata.pointer.drag(preview, strata.entry("archive"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "shelved file to arrive")
    assert not fixture.path("todo.txt").exists()
    _count(strata, 0)


def test_add_selection_then_move_explicitly(strata):
    fixture = strata.fixture
    _open_shelf(strata)
    strata.select_entry("todo.txt")
    strata.pointer.click(_action(strata, "Add selection"))
    _count(strata, 1)
    assert fixture.path("todo.txt").exists()

    strata.open_directory("archive")
    strata.pointer.click(_action(strata, "Move here"))
    strata.wait(lambda: fixture.path("archive/todo.txt").exists(), "shelved file to move")
    assert not fixture.path("todo.txt").exists()
    _count(strata, 0)
    assert fixture.path("archive/todo.txt").exists()
