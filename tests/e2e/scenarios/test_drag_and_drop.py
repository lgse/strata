# SPDX-License-Identifier: MIT
"""Moving entries by dragging them between folders."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
def test_dragging_a_file_onto_a_folder_moves_it(strata, mode):
    fixture = strata.fixture
    source = strata.select_entry("todo.txt")
    target = strata.entry("archive")

    strata.pointer.drag(source, target)

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the dragged file to arrive in archive",
    )
    strata.wait(
        lambda: not fixture.path("todo.txt").exists(),
        "the dragged file to leave its source directory",
    )
    strata.wait_for_entry_gone("todo.txt", directory=strata.fixture.root.name)
    assert fixture.path("archive/todo.txt").read_text() == "todo\n"


def test_dropping_a_file_on_itself_changes_nothing(strata):
    fixture = strata.fixture
    before = fixture.listing()
    source = strata.select_entry("todo.txt")

    strata.pointer.drag(source, source)

    strata.entry("todo.txt")
    assert fixture.listing() == before, (
        "dropping an entry on itself must not move anything"
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_dropping_a_folder_into_itself_changes_nothing(strata, mode):
    fixture = strata.fixture
    before = fixture.listing()
    source = strata.select_entry("documents")

    strata.pointer.drag(source, source)

    strata.entry("documents")
    assert fixture.listing() == before, (
        "a folder must not be moved inside itself"
    )
    assert fixture.path("documents/notes.txt").exists()


def test_releasing_outside_the_window_cancels_the_drag(strata):
    """The drag crosses a real drop target, then ends where none exists."""

    fixture = strata.fixture
    before = fixture.listing()
    source = strata.select_entry("todo.txt")
    window = strata.window.screen_bounds()
    outside = (window.x + window.width + 60, window.y + window.height + 40)

    strata.pointer.abandon_drag(source, outside)

    strata.entry("todo.txt")
    assert fixture.listing() == before, (
        "a drag released outside every drop target must not move anything"
    )


def test_dragging_onto_the_pane_background_is_a_no_op(strata):
    """Dropping an entry back into the directory it already lives in."""

    fixture = strata.fixture
    before = fixture.listing()
    source = strata.select_entry("todo.txt")
    pane = strata.pane()
    bounds = pane.screen_bounds()
    empty_point = (bounds.x + bounds.width // 2, bounds.y + bounds.height - 20)

    strata.pointer.drag_to_point(source, empty_point)

    strata.entry("todo.txt")
    assert fixture.listing() == before, (
        "dropping into the same directory must not duplicate or move anything"
    )


def test_dragging_a_folder_into_another_folder_moves_its_contents(strata):
    fixture = strata.fixture
    source = strata.select_entry("pictures")
    strata.entry("photo.txt", directory="pictures")
    strata.settle(source)
    target = strata.entry("archive")

    strata.pointer.drag(source, target)

    strata.wait(
        lambda: fixture.path("archive/pictures/photo.txt").exists(),
        "the dragged folder to arrive with its contents",
    )
    strata.wait(
        lambda: not fixture.path("pictures").exists(),
        "the dragged folder to leave its source directory",
    )
    assert sorted(fixture.names("archive/pictures")) == ["diagram.txt", "photo.txt"]


def test_dragging_a_multi_selection_moves_every_entry(strata):
    fixture = strata.fixture
    strata.select_entry("readme.md")
    strata.keyboard.press("shift+Down")
    strata.wait(
        lambda: strata.selected_names() == ["readme.md", "todo.txt"],
        "both files to be selected",
    )

    strata.pointer.drag(strata.entry("todo.txt"), strata.entry("archive"))

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists()
        and fixture.path("archive/readme.md").exists(),
        "both dragged files to arrive in archive",
    )
    assert not fixture.path("todo.txt").exists()
    assert not fixture.path("readme.md").exists()


ROW_DRAG_MODES = [
    mode for mode in ALL_MODES if mode.id != "icons"
]


@pytest.mark.parametrize("mode", ROW_DRAG_MODES)
def test_dragging_from_empty_row_space_moves_the_file(strata, mode):
    """#631: a press in the inert label allocation must start a drag, not select."""

    fixture = strata.fixture
    source = strata.entry("todo.txt")
    target = strata.entry("archive")
    start = strata.pointer.row_whitespace_point(source, "todo.txt")

    strata.pointer.drag_points(start, target.screen_bounds().center)

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the file dragged from empty row space to arrive in archive",
    )
    strata.wait(
        lambda: not fixture.path("todo.txt").exists(),
        "the file dragged from empty row space to leave its source directory",
    )


def drag_from_row_padding(strata, edge):
    fixture = strata.fixture
    source = strata.entry("todo.txt")
    target = strata.entry("archive")
    start = strata.pointer.row_padding_point(source, edge)

    strata.pointer.drag_points(start, target.screen_bounds().center)

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        f"the file dragged from {edge} row padding to arrive in archive",
    )
    strata.wait(
        lambda: not fixture.path("todo.txt").exists(),
        f"the file dragged from {edge} row padding to leave its source directory",
    )


@pytest.mark.parametrize("mode", ROW_DRAG_MODES)
@pytest.mark.parametrize("edge", ["top", "bottom"])
def test_dragging_from_row_padding_moves_the_file(strata, mode, edge):
    """#631: a press in visual row padding must reach the drag source."""

    drag_from_row_padding(strata, edge)


@pytest.mark.preferences(browser_density="airy")
@pytest.mark.parametrize("mode", ROW_DRAG_MODES)
@pytest.mark.parametrize("edge", ["top", "bottom"])
def test_dragging_from_airy_row_padding_moves_the_file(strata, mode, edge):
    drag_from_row_padding(strata, edge)
