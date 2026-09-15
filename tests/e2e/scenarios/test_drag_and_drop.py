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


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("recursive", [
    pytest.param(False, marks=pytest.mark.preferences(filter_include_subfolders=False)),
    pytest.param(True, marks=pytest.mark.preferences(filter_include_subfolders=True)),
])
def test_dragging_a_filtered_result_to_a_sidebar_folder(strata, mode, recursive):
    source_path = (
        next(strata.fixture.root.rglob("spreadsheet.csv"))
        if recursive else strata.fixture.path("todo.txt")
    )
    contents = source_path.read_bytes()
    destination = strata.environment.home / source_path.name
    strata.keyboard.press("ctrl+f")
    strata.keyboard.type_text(source_path.name)
    source = strata.wait(
        lambda: strata.window.find(role="list item", name=source_path.name),
        "the filtered drag source",
    )
    strata.pointer.drag(source, strata.sidebar_button("Home"))
    strata.wait(lambda: destination.exists(), "the filtered file to arrive in Home")
    strata.wait(lambda: not source_path.exists(), "the filtered source to be moved")
    assert destination.read_bytes() == contents
    strata.wait(
        lambda: strata.window.find(role="list item", name=source_path.name) is None,
        "the moved result to leave the filtered listing",
    )


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


@pytest.mark.preferences(single_click_previews=True)
@pytest.mark.parametrize("mode", ROW_DRAG_MODES)
def test_empty_name_space_drag_respects_view_policy(strata, mode):
    """Columns keep whole-row dragging; List name whitespace starts selection."""

    fixture = strata.fixture
    source = strata.entry("todo.txt")
    target = strata.entry("archive")
    start = strata.pointer.row_whitespace_point(source, "todo.txt")

    strata.pointer.drag_points(start, target.screen_bounds().center)

    if mode == "List":
        expect_name_space_marquee(strata)
        return
    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the file dragged from empty row space to arrive in archive",
    )
    strata.wait(
        lambda: not fixture.path("todo.txt").exists(),
        "the file dragged from empty row space to leave its source directory",
    )


def expect_name_space_marquee(strata):
    strata.wait(
        lambda: {"archive", "todo.txt"} <= set(strata.selected_names()),
        "name-column whitespace to select files rather than move one",
    )
    assert strata.fixture.path("todo.txt").exists()
    assert not strata.fixture.path("archive/todo.txt").exists()
    assert strata.preview() is None


def drag_from_row_padding(strata, mode, edge):
    fixture = strata.fixture
    source = strata.entry("todo.txt")
    target = strata.entry("archive")
    start = strata.pointer.row_padding_point(source, edge)
    if mode == "List":
        start = (strata.pointer.row_whitespace_point(source, "todo.txt")[0], start[1])

    strata.pointer.drag_points(start, target.screen_bounds().center)

    if mode == "List":
        expect_name_space_marquee(strata)
        source = strata.select_entry_with_keyboard("todo.txt")
        start = metadata_drag_origin(strata, source, edge)
        strata.pointer.drag_points(start, strata.entry("archive").screen_bounds().center)
    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        f"the file dragged from {edge} row padding to arrive in archive",
    )
    strata.wait(
        lambda: not fixture.path("todo.txt").exists(),
        f"the file dragged from {edge} row padding to leave its source directory",
    )
    assert strata.preview() is None


@pytest.mark.preferences(single_click_previews=True)
@pytest.mark.parametrize("mode", ROW_DRAG_MODES)
@pytest.mark.parametrize("edge", ["top", "bottom"])
def test_row_padding_drag_respects_view_policy(strata, mode, edge):
    drag_from_row_padding(strata, mode, edge)


@pytest.mark.preferences(
    browser_density="airy", single_click_previews=True, browser_mode="list"
)
def test_airy_row_padding_drag_respects_view_policy(strata):
    drag_from_row_padding(strata, "List", "top")


def metadata_drag_origin(strata, source, edge=None):
    metadata = next(
        label for label in source.find_all(role="label")
        if label.name and label.name != "todo.txt"
    )
    bounds = metadata.screen_bounds()
    y = bounds.center[1] if edge is None else strata.pointer.row_padding_point(source, edge)[1]
    return bounds.x + 4, y


@pytest.mark.preferences(folder_peeking=True, browser_mode="icons")
def test_starting_a_drag_cancels_a_folder_peek(strata):
    """#621: a drag beginning must cancel any open folder peek in Icons view."""

    pane = strata.pane()
    pane_bounds = pane.screen_bounds()
    strata.pointer.move_to(pane_bounds.x + 20, pane_bounds.y + pane_bounds.height - 20)

    folder = strata.entry("archive")
    start = strata.pointer.drag_origin(folder)
    strata.pointer.move_to(*start)
    strata.wait(lambda: strata.peek() is not None, "the folder peek to open on hover")

    target = strata.entry("documents")
    strata.pointer.drag_points(start, target.screen_bounds().center, release=False)
    try:
        strata.wait(
            lambda: strata.peek() is None,
            "the peek to close when the drag starts",
        )
    finally:
        strata.pointer.connection.button(1, False)


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_dragging_from_a_clipped_column_in_a_narrow_window(strata):
    """A row in a horizontally clipped column must still start a drag.

    When the window is narrow enough that two columns overflow, the newer
    column is clipped and a reveal-button overlay covers its visible strip.
    That overlay must not swallow the press meant for the row's DragSource.
    """

    fixture = strata.fixture
    strata.open_directory("documents")

    bounds = strata.window.window_bounds()
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, 600, bounds.height)
    strata.wait(lambda: strata.window.window_bounds().width == 600, "a narrow window")

    # documents is the clipped column; its visible strip sits under the
    # reveal-button overlay. Drag from it to the sidebar, whose drop target
    # is independent of the column overlay.
    source = strata.entry("notes.txt", directory="documents")
    destination = strata.environment.home / "notes.txt"

    strata.pointer.drag(source, strata.sidebar_button("Home"))

    strata.wait(lambda: destination.exists(), "the dragged file to arrive in Home")
    strata.wait(
        lambda: not fixture.path("documents/notes.txt").exists(),
        "the dragged file to leave the clipped column",
    )
    assert destination.read_text() == "notes\n"
