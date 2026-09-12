# SPDX-License-Identifier: MIT
"""Content drags, inert-space marquees, and release-only previews in every mode."""

import pytest

from harness.artifacts import ArtifactCollector
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
    drag_modifiers = (*modifiers, "alt") if mode == "Columns" else modifiers
    strata.pointer.drag_points(
        start, (end[0] + 3, end[1]), modifiers=drag_modifiers
    )

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


@pytest.mark.preferences(browser_mode="icons")
@pytest.mark.parametrize("text_size", [
    pytest.param(13, marks=pytest.mark.preferences(text_size=13)),
    pytest.param(28, marks=pytest.mark.preferences(text_size=28)),
])
@pytest.mark.parametrize("corner", ["leading", "trailing"])
def test_pane_corner_marquee_does_not_resize_sidebar(strata, text_size, corner):
    folder = _full_directory(strata)
    before = sorted(folder.iterdir())
    sidebar = strata.sidebar_button("Home").parent
    while sidebar is not None and sidebar.role != "scroll pane":
        sidebar = sidebar.parent
    assert sidebar is not None
    sidebar_before = sidebar.screen_bounds()
    pane = strata.pane().screen_bounds()
    container = strata.entry_container().screen_bounds()
    x = pane.x + 2 if corner == "leading" else container.x + container.width - 2
    start = (x, container.y + 2)
    end = strata.entry("002.txt").screen_bounds().center
    strata.pointer.drag_points(start, end)
    strata.settle(strata.pane())
    assert sidebar.screen_bounds().width == sidebar_before.width, (
        f"{text_size}px {corner} pane corner must select files, not resize the sidebar"
    )
    strata.wait(
        lambda: "002.txt" in strata.selected_names() and len(strata.selected_names()) > 1,
        "a marquee from the pane corner to select files",
    )
    selected = strata.selected_names()
    divider = ((sidebar_before.x + sidebar_before.width + pane.x) // 2, container.center[1])
    strata.pointer.drag_points(divider, (divider[0] + 40, divider[1]))
    strata.wait(
        lambda: sidebar.screen_bounds().width >= sidebar_before.width + 30,
        "dragging the actual sidebar divider to resize it",
    )
    assert strata.selected_names() == selected
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
def test_ctrl_drag_from_selected_content_copies_the_file(strata, mode):
    strata.select_entry("todo.txt")
    start = strata.pointer.drag_origin(strata.entry("todo.txt"))
    target = strata.entry("archive")
    strata.pointer.drag_points(
        start, target.screen_bounds().center, modifiers=("ctrl",)
    )
    strata.wait(
        lambda: strata.fixture.path("archive/todo.txt").exists(),
        "the ctrl-drag from selected content to copy the file",
    )
    assert strata.fixture.path("todo.txt").exists()


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
@pytest.mark.parametrize("focus_origin", ["pane", "sidebar"])
def test_sidebar_marquee_still_reaches_the_leading_pane(strata, mode, focus_origin):
    root = strata.fixture.root.name
    if mode == "Columns":
        strata.open_directory("documents")
    home = strata.sidebar_button("Home")
    if focus_origin == "sidebar":
        for _ in range(2):
            strata.keyboard.press("Home")
            strata.keyboard.press("Left")
            if home.has_state("focused"):
                break
        strata.wait(lambda: home.has_state("focused"), "keyboard focus in the sidebar")
    sidebar = home.parent
    assert sidebar is not None
    bounds = sidebar.screen_bounds()
    start = (bounds.center[0], bounds.y + bounds.height - 10)
    first = strata.entry("readme.md", root).screen_bounds().center
    last = strata.entry("todo.txt", root).screen_bounds().center
    strata.pointer.drag_points(start, (last[0], first[1]), release=False)
    try:
        strata.wait(
            lambda: {"readme.md", "todo.txt"} <= set(strata.selected_names(root)),
            "the sidebar marquee to select in the leading pane",
        )
        collection = strata.entry_container(root)
        strata.wait(
            lambda: any(node.has_state("focused") for _, node in collection.walk()),
            "the marquee target to own focus and active selection feedback during the drag",
        )
        strata.screenshot(
            ArtifactCollector(test_name=f"sidebar-marquee-{mode}-{focus_origin}").directory
            / "selection.png"
        )
    finally:
        strata.pointer.connection.button(1, False)
    assert not home.has_state("focused")
    selected = set(strata.selected_names(root))
    strata.keyboard.press("ctrl+c")
    destination = "selection-copy"
    strata.fixture.path(destination).mkdir()
    strata.open_directory(destination, root)
    strata.paste_into(destination)
    strata.wait(
        lambda: set(strata.fixture.names(destination)) == selected,
        "keyboard copy to use the marquee target rather than the sidebar or another pane",
    )
    for name in ("readme.md", "todo.txt"):
        assert strata.fixture.path(f"{destination}/{name}").read_bytes() == strata.fixture.path(
            name
        ).read_bytes()


@pytest.mark.parametrize("mode", ALL_MODES)
def test_marquee_from_a_full_row_auto_scrolls(strata, mode):
    _full_directory(strata)
    start = _inert_point(strata, "000.txt", mode)
    pane = strata.pane().screen_bounds()
    container = strata.entry_container().screen_bounds()
    bottom = min(pane.y + pane.height, container.y + container.height)
    modifiers = ("alt",) if mode == "Columns" else ()
    strata.pointer.drag_points(
        start, (start[0] + 3, bottom - 4), release=False, modifiers=modifiers
    )
    try:
        strata.wait(
            lambda: any(name >= "060.txt" for name in strata.selected_names()),
            "edge auto-scroll to extend selection beyond the initial viewport",
        )
    finally:
        strata.pointer.connection.button(1, False)
    assert strata.preview() is None
