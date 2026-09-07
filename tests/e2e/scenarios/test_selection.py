# SPDX-License-Identifier: GPL-3.0-or-later
"""Range selection, toggle selection, and right-click selection behavior."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES


@pytest.fixture
def root(strata) -> str:
    """The fixture directory, named explicitly.

    Columns opens a folder on the click that selects it, so assertions name
    the pane they are about rather than relying on the deepest one.
    """

    return strata.fixture.root.name


@pytest.mark.parametrize("mode", ALL_MODES)
def test_shift_click_ranges_from_the_initial_listing(strata, mode, root):
    strata.click_entry_with("pictures", ["shift"], directory=root)
    strata.wait_for_selection(["archive", "documents", "pictures"], root)


@pytest.mark.parametrize("mode", ALL_MODES)
def test_sidebar_navigation_initializes_the_range_anchor(strata, mode):
    home = strata.environment.home
    names = ["a.txt", "b.txt", "c.txt"]
    for name in names:
        (home / name).write_text(name)
    strata.pointer.click(strata.sidebar_button("Home"))
    strata.wait_for_directory(home.name)
    strata.wait_for_selection(["a.txt"], home.name)
    strata.click_entry_with("c.txt", ["shift"], directory=home.name)
    strata.wait_for_selection(names, home.name)


@pytest.mark.preferences(browser_mode="columns")
def test_returning_to_a_parent_pane_anchors_its_first_entry(strata, root):
    strata.open_directory("documents", directory=root)
    strata.pointer.click(strata.pane(root), at=strata.background_point(root))
    strata.wait_for_selection(["archive"], root)
    strata.click_entry_with("pictures", ["shift"], directory=root)
    strata.wait_for_selection(["archive", "documents", "pictures"], root)
    assert "documents" in strata.pane_names()


@pytest.mark.parametrize("mode", ALL_MODES)
def test_shift_click_selects_a_range(strata, mode, root):
    strata.select_entry("archive", directory=root)

    strata.click_entry_with("pictures", ["shift"], directory=root)

    strata.wait(
        lambda: strata.selected_names(root) == ["archive", "documents", "pictures"],
        "a shift-click to select the whole range",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_control_click_toggles_individual_entries(strata, mode, root):
    strata.select_entry("archive", directory=root)

    strata.click_entry_with("pictures", ["ctrl"], directory=root)
    strata.wait(
        lambda: strata.selected_names(root) == ["archive", "pictures"],
        "a control-click to add one entry",
    )

    strata.click_entry_with("archive", ["ctrl"], directory=root)
    strata.wait(
        lambda: strata.selected_names(root) == ["pictures"],
        "a second control-click to remove that entry again",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_right_click_selects_the_entry_under_the_pointer(strata, mode, root):
    strata.select_entry("readme.md", directory=root)

    strata.open_context_menu("todo.txt", directory=root)

    strata.wait(
        lambda: strata.selected_names(root) == ["todo.txt"],
        "the right-clicked entry to become the selection",
    )
    strata.dismiss_menu()


def test_right_click_keeps_an_existing_multi_selection(strata, root):
    strata.select_entry("readme.md", directory=root)
    strata.click_entry_with("todo.txt", ["ctrl"], directory=root)
    strata.wait(
        lambda: strata.selected_names(root) == ["readme.md", "todo.txt"],
        "both files to be selected",
    )

    strata.open_context_menu("todo.txt", directory=root)

    assert strata.selected_names(root) == ["readme.md", "todo.txt"], (
        "right-clicking inside a multi-selection must not collapse it"
    )
    strata.dismiss_menu()


@pytest.mark.parametrize("mode", ALL_MODES)
def test_selecting_a_second_entry_replaces_the_first(strata, mode, root):
    strata.select_entry("readme.md", directory=root)

    strata.select_entry("todo.txt", directory=root)

    strata.wait(
        lambda: strata.selected_names(root) == ["todo.txt"],
        "a plain click to replace the selection",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_shift_click_ranges_from_the_entry_a_fresh_listing_selected(strata, mode, root):
    strata.open_directory("documents", directory=root)

    strata.click_entry_with("spreadsheet.csv", ["shift"], directory="documents")

    strata.wait(
        lambda: strata.selected_names("documents")
        == ["notes.txt", "report.md", "spreadsheet.csv"],
        "a shift-click to range from the entry the listing selected on load",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_a_click_after_navigating_re_anchors_the_range(strata, mode, root):
    strata.open_directory("documents", directory=root)
    strata.select_entry("report.md", directory="documents")

    strata.click_entry_with("spreadsheet.csv", ["shift"], directory="documents")

    strata.wait(
        lambda: strata.selected_names("documents") == ["report.md", "spreadsheet.csv"],
        "the range to start at the clicked entry rather than the loaded one",
    )
