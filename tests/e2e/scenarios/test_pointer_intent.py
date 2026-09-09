# SPDX-License-Identifier: MIT
"""Content drags, inert-space marquees, and release-only previews in every mode."""

import pytest

from harness.modes import ALL_MODES


@pytest.mark.preferences(single_click_previews=True)
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("origin", ["icon", "name"])
def test_file_drag_does_not_open_preview(strata, mode, origin):
    source = strata.entry("todo.txt")
    if origin == "icon":
        start = strata.pointer.drag_origin(source)
    else:
        label = source.find(role="label", name="todo.txt")
        assert label is not None
        bounds = label.screen_bounds()
        start = bounds.center if mode == "Icons" else (bounds.x + 4, bounds.center[1])
    target = strata.entry("archive")

    def assert_no_preview_on_press():
        strata.entry("todo.txt")
        assert strata.preview() is None, "a held press must not open a preview"

    strata.pointer.drag_points(
        start, target.screen_bounds().center, release=False,
        after_press=assert_no_preview_on_press,
    )
    try:
        assert strata.preview() is None, "crossing the drag threshold must suppress preview"
    finally:
        strata.pointer.connection.button(1, False)
    strata.wait(lambda: strata.fixture.path("archive/todo.txt").exists(), "the file drop")
    assert not strata.fixture.path("todo.txt").exists()
    assert strata.preview() is None


@pytest.mark.preferences(single_click_previews=True)
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("origin", ["content", "inert"])
def test_simple_click_still_opens_preview(strata, mode, origin):
    at = _inert_point(strata, "todo.txt", mode) if origin == "inert" else None
    strata.pointer.click(strata.entry("todo.txt"), at=at)
    strata.wait(lambda: strata.preview_shows("todo"), "preview after a simple click")


def _full_directory(strata):
    folder = strata.fixture.path("full")
    folder.mkdir()
    for index in range(180):
        (folder / f"{index:03}.txt").write_text(f"{index}\n")
    strata.open_directory("full")
    return folder


def _inert_point(strata, name, mode):
    row = strata.entry(name)
    if mode == "Icons":
        icon = row.find(role="image")
        assert icon is not None
        bounds = icon.screen_bounds()
        return bounds.x - 6, bounds.center[1]
    label = row.find(role="label", name=name)
    assert label is not None
    bounds = label.screen_bounds()
    return bounds.x + bounds.width * 2 // 3, bounds.center[1]


@pytest.mark.preferences(single_click_previews=True)
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("modifiers", [(), ("ctrl",), ("shift",)])
def test_marquee_begins_beside_content_in_a_full_pane(strata, mode, modifiers):
    folder = _full_directory(strata)
    before = sorted(folder.iterdir())
    initial = set(strata.selected_names())
    start = _inert_point(strata, "000.txt", mode)
    end = _inert_point(strata, "010.txt", mode)
    strata.pointer.drag_points(start, (end[0] + 3, end[1]), modifiers=modifiers)

    strata.wait(
        lambda: len(strata.selected_names()) > 1,
        "marquee selection beside occupied rows",
    )
    selected = set(strata.selected_names())
    for name in ("000.txt", "010.txt"):
        expected = name not in initial if "ctrl" in modifiers else True
        assert (name in selected) == expected
    assert strata.preview() is None
    assert sorted(folder.iterdir()) == before


@pytest.mark.parametrize("mode", ALL_MODES)
def test_modifier_clicks_on_inert_space_still_select(strata, mode):
    _full_directory(strata)
    strata.select_entry("000.txt")
    strata.pointer.click(
        strata.entry("002.txt"),
        at=_inert_point(strata, "002.txt", mode),
        modifiers=("ctrl",),
    )
    strata.wait_for_selection(["000.txt", "002.txt"])
    strata.pointer.click(
        strata.entry("004.txt"),
        at=_inert_point(strata, "004.txt", mode),
        modifiers=("shift",),
    )
    strata.wait_for_selection(["002.txt", "003.txt", "004.txt"])


@pytest.mark.parametrize("mode", ALL_MODES)
def test_ctrl_drag_from_content_copies_and_keeps_selection(strata, mode):
    # Not pre-selected: a ctrl-press on an *unselected* file adds it to the
    # selection, so the drag that follows has an unambiguous selection to
    # preserve. (Ctrl-dragging an already-selected file toggles it off on
    # press before any drag starts — separate, pre-existing behavior.)
    start = strata.pointer.drag_origin(strata.entry("todo.txt"))
    target = strata.entry("archive")
    strata.pointer.drag_points(
        start, target.screen_bounds().center, modifiers=("ctrl",)
    )
    strata.wait(
        lambda: strata.fixture.path("archive/todo.txt").exists(),
        "the ctrl-drag from content to copy the file",
    )
    assert strata.fixture.path("todo.txt").exists()
    strata.wait_for_selection(["todo.txt"])


@pytest.mark.parametrize("mode", ALL_MODES)
def test_shift_drag_from_content_moves_the_file(strata, mode):
    start = strata.pointer.drag_origin(strata.entry("todo.txt"))
    target = strata.entry("archive")
    strata.pointer.drag_points(
        start, target.screen_bounds().center, modifiers=("shift",)
    )
    strata.wait(
        lambda: strata.fixture.path("archive/todo.txt").exists(),
        "the shift-drag from content to move the file",
    )
    assert not strata.fixture.path("todo.txt").exists()


@pytest.mark.parametrize("mode", ALL_MODES)
def test_sidebar_marquee_still_reaches_the_leading_pane(strata, mode):
    root = strata.fixture.root.name
    if mode == "Columns":
        strata.open_directory("documents")
    sidebar = strata.sidebar_button("Home").parent
    assert sidebar is not None
    bounds = sidebar.screen_bounds()
    start = (bounds.center[0], bounds.y + bounds.height - 10)
    first = strata.entry("readme.md", root).screen_bounds().center
    last = strata.entry("todo.txt", root).screen_bounds().center
    strata.pointer.drag_points(start, (last[0], first[1]))
    strata.wait(
        lambda: {"readme.md", "todo.txt"} <= set(strata.selected_names(root)),
        "the sidebar marquee to select in the leading pane",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_marquee_from_a_full_row_auto_scrolls(strata, mode):
    _full_directory(strata)
    start = _inert_point(strata, "000.txt", mode)
    pane = strata.pane().screen_bounds()
    container = strata.entry_container().screen_bounds()
    bottom = min(pane.y + pane.height, container.y + container.height)
    end = (start[0] + 3, bottom - 4)
    strata.pointer.drag_points(start, end, release=False)
    try:
        strata.wait(
            lambda: any(name >= "060.txt" for name in strata.selected_names()),
            "edge auto-scroll to extend selection beyond the initial viewport",
        )
    finally:
        strata.pointer.connection.button(1, False)
    assert strata.preview() is None
