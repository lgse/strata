# SPDX-License-Identifier: MIT
"""Empty-space clicks clear selection without taking over item clicks or drags."""

import pytest

from harness.modes import ALL_MODES


def _select_files(strata):
    root = strata.fixture.root.name
    strata.select_entry("readme.md", root)
    strata.click_entry_with("todo.txt", ["ctrl"], root)
    strata.wait_for_selection(["readme.md", "todo.txt"], root)
    return root


def _blank_point(strata, directory):
    bounds = strata.pane(directory).screen_bounds()
    return bounds.center[0], bounds.y + bounds.height - 60


@pytest.mark.parametrize("mode", ALL_MODES)
def test_background_click_clears_all_selection(strata, mode):
    root = _select_files(strata)
    strata.pointer.click(strata.pane(root), at=_blank_point(strata, root))
    strata.wait(lambda: not strata.all_selected_names(), "empty space to deselect every file")
    assert strata.pane().name == root


@pytest.mark.parametrize("mode", ALL_MODES)
def test_background_press_does_not_clear_before_drag_intent(strata, mode):
    root = _select_files(strata)
    start = _blank_point(strata, root)
    end = strata.entry("todo.txt", root).screen_bounds().center

    def still_selected():
        assert strata.selected_names(root) == ["readme.md", "todo.txt"]

    strata.pointer.drag_points(start, end, after_press=still_selected)
    strata.wait(lambda: bool(strata.selected_names(root)), "marquee selection to survive release")


def test_clicking_an_empty_column_clears_other_columns_without_closing_them(strata):
    strata.open_directory("archive")
    _select_files(strata)
    panes = strata.pane_names()
    strata.pointer.click(strata.pane("archive"), at=_blank_point(strata, "archive"))
    strata.wait(lambda: not strata.all_selected_names(), "empty column to clear all selections")
    assert strata.pane_names() == panes
    assert strata.current_directory() == "archive"


def test_clicking_beside_the_last_column_clears_selection(strata):
    root = _select_files(strata)
    window = strata.window.screen_bounds()
    pane = strata.pane(root).screen_bounds()
    point = (window.x + window.width - 40, pane.center[1])
    strata.pointer.click(strata.window, at=point)
    strata.wait(lambda: not strata.all_selected_names(), "blank space beside Columns to deselect")
