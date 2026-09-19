# SPDX-License-Identifier: MIT
"""Moving entries by dragging them between folders."""

from __future__ import annotations

import pytest

from harness.browser import ENTRY_ROLES
from harness.modes import ALL_MODES


def _scroll_pane(node):
    for ancestor in node.ancestors():
        if ancestor.role == "scroll pane":
            return ancestor
    raise AssertionError("the node should sit inside a scroll viewport")


def _entry_name(row):
    label = next((node for _, node in row.walk() if node.role == "label"), None)
    return label.name if label is not None and label.name else row.name


def _visible_rows(container, viewport):
    viewport_bounds = viewport.screen_bounds()
    origin = container.screen_bounds().y - container.window_bounds().y
    for row in container.children:
        if row.role not in ENTRY_ROLES:
            continue
        bounds = row.window_bounds()
        if (
            bounds.height > 0
            and bounds.y + origin >= viewport_bounds.y
            and bounds.y + origin + bounds.height <= viewport_bounds.y + viewport_bounds.height
        ):
            yield row


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
    fixture = strata.fixture
    strata.open_directory("documents")

    bounds = strata.window.window_bounds()
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, 600, bounds.height)
    strata.wait(lambda: strata.window.window_bounds().width == 600, "a narrow window")

    source = strata.entry("notes.txt", directory="documents")
    destination = strata.environment.home / "notes.txt"

    strata.pointer.drag(source, strata.sidebar_button("Home"))

    strata.wait(lambda: destination.exists(), "the dragged file to arrive in Home")
    strata.wait(
        lambda: not fixture.path("documents/notes.txt").exists(),
        "the dragged file to leave the clipped column",
    )
    assert destination.read_text() == "notes\n"


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_dragging_a_file_to_the_strip_edge_scrolls_columns_in(strata):
    fixture = strata.fixture
    nested = fixture.path("documents/deep/deeper")
    nested.mkdir(parents=True)
    (nested / "cargo.txt").write_text("cargo\n")

    bounds = strata.window.window_bounds()
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, 640, 360)
    strata.wait(lambda: strata.window.window_bounds().width == 640, "a narrow window")

    strata.open_directory("documents")
    strata.open_directory("deep", directory="documents")
    strata.open_directory("deeper", directory="deep")
    strata.settle(strata.pane("deeper"))

    strip = _scroll_pane(strata.pane("deeper")).screen_bounds()
    listing = _scroll_pane(strata.entry_container("deeper")).screen_bounds()
    edge = (strip.x + 6, listing.y + listing.height - 4)

    source = strata.entry("cargo.txt", "deeper")
    strata.pointer.drag_points(strata.pointer.drag_origin(source), edge, release=False)
    try:
        root = fixture.root.name

        def root_column_under_pointer():
            for pane in strata.containers():
                if pane.name == root:
                    bounds = pane.screen_bounds()
                    if bounds.x <= edge[0] < bounds.x + bounds.width:
                        return pane
            return None

        strata.wait(
            root_column_under_pointer,
            "the parked drag to edge-scroll the root column under the pointer",
        )
        # Content moved under the held pointer; a nudge refreshes the drop site.
        strata.pointer.move_to(edge[0], edge[1] - 4)
    finally:
        strata.pointer.connection.button(1, False)

    strata.wait(
        lambda: fixture.path("cargo.txt").exists(),
        "the release to drop the file into the revealed root column",
    )
    assert not (nested / "cargo.txt").exists()


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_dragging_a_file_to_the_column_edge_scrolls_its_listing(strata):
    fixture = strata.fixture
    documents = fixture.path("documents")
    for index in range(300):
        (documents / f"{index:03}.txt").write_text("x\n")
    strata.open_directory("documents")

    container = strata.entry_container("documents")
    assert container is not None
    viewport = _scroll_pane(container)
    bounds = viewport.screen_bounds()

    source = strata.entry("000.txt", "documents")
    strata.pointer.drag_points(
        strata.pointer.drag_origin(source),
        (bounds.center[0], bounds.y + bounds.height - 8),
        release=False,
    )
    try:
        strata.wait(
            lambda: any(
                _entry_name(row) >= "040.txt"
                for row in _visible_rows(container, viewport)
            ),
            "the parked drag to edge-scroll the column's listing",
        )
    finally:
        strata.pointer.connection.button(1, False)

    strata.wait(
        lambda: fixture.path("documents/000.txt").exists(),
        "the file to stay in its own directory after the drag",
    )


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_dropping_on_partially_visible_column_moves_file_without_jumping(strata):
    fixture = strata.fixture
    deep = fixture.path("documents/deep")
    deep.mkdir(parents=True)
    (deep / "cargo.txt").write_text("cargo\n")

    bounds = strata.window.window_bounds()
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, 640, 360)
    strata.wait(lambda: strata.window.window_bounds().width == 640, "a narrow window")

    strata.open_directory("documents")
    strata.open_directory("deep", directory="documents")
    strata.settle(strata.pane("deep"))

    documents_pane = strata.pane("documents")
    assert documents_pane is not None

    source = strata.entry("cargo.txt", "deep")
    doc_bounds = documents_pane.screen_bounds()
    strip = _scroll_pane(strata.pane("deep")).screen_bounds()
    drop_x = max(doc_bounds.x + 10, strip.x + 10)
    drop_y = doc_bounds.y + doc_bounds.height // 2

    strata.pointer.drag_points(
        strata.pointer.drag_origin(source),
        (drop_x, drop_y),
        release=True,
    )

    strata.wait(
        lambda: fixture.path("documents/cargo.txt").exists(),
        "the file to arrive in documents directory",
    )
    assert not (deep / "cargo.txt").exists()
    assert strata.pane("deep") is not None
