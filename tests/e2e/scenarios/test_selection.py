# SPDX-License-Identifier: MIT
"""Range selection, toggle selection, and right-click selection behavior."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES

ROW_MODES = [mode for mode in ALL_MODES if mode.id != "icons"]


def _name_label_point(entry, *, leftover: bool) -> tuple[int, int]:
    label = entry.find(role="label", name=entry.name)
    assert label is not None
    bounds = label.screen_bounds()
    x = bounds.x + bounds.width - 2 if leftover else bounds.x + 4
    return (x, bounds.center[1])


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
def test_keyboard_selection_after_sidebar_navigation_initializes_the_range_anchor(strata, mode):
    home = strata.environment.home
    names = ["a.txt", "b.txt", "c.txt"]
    for name in names:
        (home / name).write_text(name)
    strata.pointer.click(strata.sidebar_button("Home"))
    strata.wait_for_directory(home.name)
    strata.keyboard.press("Home")
    strata.wait_for_selection(["a.txt"], home.name)
    strata.click_entry_with("c.txt", ["shift"], directory=home.name)
    strata.wait_for_selection(names, home.name)


@pytest.mark.preferences(
    browser_mode="columns", single_click_previews=False,
    sort_key="modified", sort_direction="descending",
)
@pytest.mark.parametrize("target", ["name", "row-space"])
def test_shift_click_revisits_a_file_after_opening_a_folder(strata, root, target):
    def click(name, modifiers=()):
        entry = strata.entry(name, root)
        label = entry.find(role="label", name=name)
        assert label is not None
        bounds = label.screen_bounds()
        x = bounds.x + 4 if target == "name" else bounds.x + bounds.width - 2
        strata.pointer.click(entry, at=(x, bounds.center[1]), modifiers=modifiers)

    click("todo.txt")
    strata.wait_for_selection(["todo.txt"], root)
    click("documents")
    strata.wait_for_directory("documents")
    strata.wait_for_selection([], "documents")
    click("todo.txt", ("shift",))
    if target == "name":
        strata.wait_for_focused_entry("todo.txt")
    names = [entry.name for entry in strata.entries(root)]
    strata.wait_for_selection(names[names.index("documents"):names.index("todo.txt") + 1], root)


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("modifier", ["ctrl", "shift"])
def test_modifier_click_on_a_filename_focuses_the_target(strata, mode, modifier, root):
    strata.select_entry("readme.md", root)
    strata.wait_for_focused_entry("readme.md")
    entry = strata.entry("todo.txt", root)
    label = entry.find(role="label", name="todo.txt")
    assert label is not None
    bounds = label.screen_bounds()
    strata.pointer.click(entry, at=(bounds.x + 4, bounds.center[1]), modifiers=(modifier,))
    strata.wait_for_focused_entry("todo.txt")
    strata.wait_for_selection(["readme.md", "todo.txt"], root)


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
@pytest.mark.parametrize("target", ["content", "row-space"])
def test_shift_click_ranges_from_the_entry_a_fresh_listing_selected(strata, mode, root, target):
    strata.select_entry_with_keyboard("documents")
    strata.keyboard.press("Return")
    strata.wait_for_directory("documents")

    entry = strata.entry("spreadsheet.csv", "documents")
    point = None
    if target == "row-space":
        if mode == "Icons":
            icon = entry.find(role="image")
            assert icon is not None
            bounds = icon.screen_bounds()
            point = (bounds.x - 6, bounds.center[1])
        else:
            label = entry.find(role="label", name="spreadsheet.csv")
            assert label is not None
            bounds = label.screen_bounds()
            point = (bounds.x + bounds.width - 2, bounds.center[1])
    strata.pointer.click(entry, at=point, modifiers=("shift",))

    strata.wait(
        lambda: strata.selected_names("documents")
        == ["notes.txt", "report.md", "spreadsheet.csv"],
        "a shift-click to range from the entry the listing selected on load",
    )


@pytest.mark.parametrize("mode", ROW_MODES)
@pytest.mark.parametrize("target", ["content", "row-space"])
def test_shift_click_keeps_range_when_shift_releases_before_mouseup(
    strata, mode, root, target
):
    strata.open_directory("documents", directory=root)
    strata.select_entry("notes.txt", directory="documents")

    entry = strata.entry("spreadsheet.csv", "documents")
    point = _name_label_point(entry, leftover=(target == "row-space"))
    strata.pointer.click_releasing_modifiers_before_up(
        entry, at=point, modifiers=("shift",)
    )

    strata.wait_for_selection(
        ["notes.txt", "report.md", "spreadsheet.csv"], "documents"
    )


@pytest.mark.preferences(browser_mode="columns")
def test_control_click_leftover_toggles_when_ctrl_releases_before_mouseup(strata, root):
    strata.open_directory("documents", directory=root)
    strata.select_entry("notes.txt", directory="documents")

    entry = strata.entry("spreadsheet.csv", "documents")
    strata.pointer.click_releasing_modifiers_before_up(
        entry,
        at=_name_label_point(entry, leftover=True),
        modifiers=("ctrl",),
    )

    strata.wait_for_selection(["notes.txt", "spreadsheet.csv"], "documents")


@pytest.mark.parametrize("mode", ROW_MODES)
def test_unmodified_leftover_click_replaces_the_selection(strata, mode, root):
    strata.open_directory("documents", directory=root)
    strata.select_entry("notes.txt", directory="documents")

    entry = strata.entry("spreadsheet.csv", "documents")
    strata.pointer.click(
        entry, at=_name_label_point(entry, leftover=True)
    )

    strata.wait_for_selection(["spreadsheet.csv"], "documents")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_a_click_after_navigating_re_anchors_the_range(strata, mode, root):
    strata.open_directory("documents", directory=root)
    strata.select_entry("report.md", directory="documents")

    strata.click_entry_with("spreadsheet.csv", ["shift"], directory="documents")

    strata.wait(
        lambda: strata.selected_names("documents") == ["report.md", "spreadsheet.csv"],
        "the range to start at the clicked entry rather than the loaded one",
    )
