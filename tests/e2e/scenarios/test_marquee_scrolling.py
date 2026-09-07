# SPDX-License-Identifier: GPL-3.0-or-later
"""Scrolling must extend a held marquee, not move its anchor or lose earlier hits."""

import pytest

from harness.modes import ALL_MODES


def _viewport(strata):
    parent = strata.entry_container().parent
    while parent is not None:
        if parent.role == "scroll pane":
            return parent.screen_bounds()
        parent = parent.parent
    raise AssertionError("the collection should have a scroll viewport")


def _visible_entries(strata, viewport):
    visible = []
    for row in strata.entries():
        bounds = row.screen_bounds()
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
    viewport = _viewport(strata)
    end = (
        viewport.x + viewport.width - 24,
        viewport.y
        + (viewport.height - 8 if scrolling == "edge" else viewport.height * 4 // 5),
    )
    strata.pointer.drag_points(start, end, release=False)
    try:
        if scrolling == "wheel":
            strata.pointer.scroll(at=end, clicks=32)
        strata.wait(
            lambda: any(
                row.name >= "060.txt" for row in _visible_entries(strata, viewport)
            ),
            f"scrolling to carry the anchor above the viewport {viewport}",
        )

        if scrolling == "edge":
            end = (end[0], viewport.y + viewport.height - 40)
            strata.pointer.move_to(*end)
        strata.settle(_visible_entries(strata, viewport)[0])

        def visible_band_is_selected():
            rows = []
            for row in _visible_entries(strata, viewport):
                bounds = row.screen_bounds()
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
        if any(row.name == "000.txt" for row in _visible_entries(strata, viewport)):
            break
        strata.pointer.scroll(at=viewport.center, clicks=20, down=False)
    strata.wait(
        lambda: any(row.name == "000.txt" for row in _visible_entries(strata, viewport)),
        "the beginning of the directory to scroll back into view",
    )
    assert strata.entry("010.txt").has_state("selected"), (
        "scrolling must retain the original selection"
    )
    assert not strata.entry("000.txt").has_state("selected"), (
        "files above the anchor must stay unselected"
    )
