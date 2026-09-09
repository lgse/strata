# SPDX-License-Identifier: MIT
"""Scrolling must extend a held marquee, not move its anchor or lose earlier hits."""

import pytest

from harness.modes import ALL_MODES


def _viewport(container):
    parent = container.parent
    while parent is not None:
        if parent.role == "scroll pane":
            return parent
        parent = parent.parent
    raise AssertionError("the collection should have a scroll viewport")


def _entry_bounds(row):
    label = row.find(role="label")
    return label.screen_bounds() if label is not None else row.screen_bounds()


def _entry_name(row):
    label = row.find(role="label")
    return label.name if label is not None and label.name else row.name


def _visible_entries(strata, viewport):
    viewport = viewport.screen_bounds()
    visible = []
    for row in strata.entries():
        bounds = _entry_bounds(row)
        if (
            bounds.height > 0
            and bounds.y >= viewport.y
            and bounds.y + bounds.height <= viewport.y + viewport.height
        ):
            visible.append(row)
    return visible


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("scrolling", ["edge", "wheel"])
def test_scrolling_extends_marquee_without_losing_earlier_files(strata, mode, scrolling):
    folder = strata.fixture.path("scrolling")
    folder.mkdir()
    for index in range(600):
        (folder / f"{index:03}.txt").write_text(f"{index}\n")
    strata.open_directory("scrolling")
    anchor = strata.entry("010.txt")
    if mode == "Icons":
        icon = anchor.find(role="image")
        assert icon is not None
        bounds = icon.screen_bounds()
        start = (bounds.x - 6, bounds.center[1])
    else:
        label = anchor.find(role="label", name="010.txt")
        assert label is not None
        bounds = label.screen_bounds()
        start = (bounds.x + bounds.width * 2 // 3, bounds.center[1])
    container = strata.entry_container()
    viewport = _viewport(container)
    viewport_bounds = viewport.screen_bounds()
    end = (
        viewport_bounds.x + viewport_bounds.width - 24,
        viewport_bounds.y
        + (
            viewport_bounds.height - 8
            if scrolling == "edge"
            else viewport_bounds.height * 4 // 5
        ),
    )
    strata.pointer.drag_points(start, end, release=False)
    try:
        if scrolling == "wheel":
            strata.pointer.scroll(at=end, clicks=32)
        strata.wait(
            lambda: any(
                _entry_name(row) >= "060.txt" for row in _visible_entries(strata, viewport)
            ),
            f"scrolling to carry the anchor above the viewport {viewport_bounds}",
        )

        if scrolling == "edge":
            end = (end[0], viewport_bounds.y + viewport_bounds.height - 40)
            strata.pointer.move_to(*end)
        strata.settle(_visible_entries(strata, viewport)[0])

        def visible_band_is_selected():
            rows = []
            for row in _visible_entries(strata, viewport):
                bounds = _entry_bounds(row)
                if (
                    bounds.y + bounds.height <= end[1]
                    and bounds.x < end[0]
                    and bounds.x + bounds.width > start[0]
                ):
                    rows.append(row)
            return rows and all(row.has_state("selected") for row in rows)

        strata.wait(
            visible_band_is_selected,
            "every visible row inside the scrolled marquee to be selected",
        )
    finally:
        strata.pointer.connection.button(1, False)

    for _ in range(40):
        if any(_entry_name(row) == "000.txt" for row in _visible_entries(strata, viewport)):
            break
        strata.pointer.scroll(at=viewport_bounds.center, clicks=20, down=False)
    strata.wait(
        lambda: any(_entry_name(row) == "000.txt" for row in _visible_entries(strata, viewport)),
        "the beginning of the directory to scroll back into view",
    )
    assert strata.entry("010.txt").has_state("selected"), (
        "scrolling must retain the original selection"
    )
    assert not strata.entry("000.txt").has_state("selected"), (
        "files above the anchor must stay unselected"
    )
