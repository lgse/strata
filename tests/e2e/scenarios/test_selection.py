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
