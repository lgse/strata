# SPDX-License-Identifier: GPL-3.0-or-later
"""After compression completes, the new archive is selected and scrolled into view."""

import pytest

from harness import tree
from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES


@pytest.fixture
def fixture_tree():
    files = {f"{index:03}.txt": f"{index}\n" for index in range(200)}
    fixture = FixtureTree.create(
        {"nested": files, "a": {"b": {"c": {"d": files}}}, **files}
    )
    try:
        yield fixture
    finally:
        fixture.cleanup()


def _compress(strata, entry_name, archive_name, directory, folder=""):
    for _ in range(3):
        strata.open_context_menu(entry_name, directory=directory)
        # Coordinate-free activation: the popover can still be settling when
        # the item is found, and a synthetic click then lands outside it.
        strata.menu_item("Compress…").activate()
        strata.wait_for_menu_closed()
        field = None
        try:
            field = strata.wait(
                lambda: strata.window.find(role="text", states={"editable", "focused"}),
                "the compress name field to take focus",
                timeout=5,
            )
        except tree.TreeTimeout:
            pass
        if field is not None:
            strata.keyboard.press("ctrl+a")
            strata.keyboard.type_text(archive_name)
            strata.wait(
                lambda: field.text == archive_name,
                f"{archive_name!r} to reach the name field",
            )
            strata.keyboard.press("Return")
            strata.wait(lambda: strata.dialog() is None, "the dialog to close")
            path = strata.fixture.path(folder).joinpath(f"{archive_name}.zip")
            strata.wait(lambda: path.exists(), "the archive to be created")
            return
    raise AssertionError("the compress dialog never accepted the archive name")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_archive_far_from_viewport_is_scrolled_into_view(strata, mode):
    """A custom archive name that sorts far from the viewport still gets revealed."""
    strata.wait_for_view(mode)

    _compress(strata, "005.txt", "zzz", None)

    strata.wait_for_selection(["zzz.zip"])
    entry = strata.entry("zzz.zip")
    strata.wait(lambda: strata.on_screen(entry), "the distant archive to be on screen")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_compressed_archive_is_revealed(strata, mode):
    """The archive lands at the end of the listing and must be scrolled into view."""
    strata.wait_for_view(mode)
    strata.keyboard.press("End")
    strata.wait_for_focused_entry("199.txt")

    _compress(strata, "199.txt", "199", None)

    strata.wait_for_selection(["199.zip"])
    entry = strata.entry("199.zip")
    strata.wait(lambda: strata.on_screen(entry), "the new archive to be on screen")


def test_columns_reveals_archive_in_active_child_column(strata):
    """Columns mode scrolls the archive into view inside the open child column."""
    strata.wait_for_view("Columns")
    strata.open_directory("nested")
    strata.keyboard.press("End")
    strata.wait_for_focused_entry("199.txt")

    _compress(strata, "199.txt", "199", "nested", folder="nested")

    strata.wait_for_selection(["199.zip"], directory="nested")
    entry = strata.entry("199.zip", directory="nested")
    strata.wait(
        lambda: strata.on_screen(entry),
        "the archive to be on screen in the child column",
    )


def test_columns_reveals_archive_in_deeply_nested_column(strata):
    """Columns mode scrolls horizontally and vertically to the archive in a deep column."""
    strata.wait_for_view("Columns")
    strata.open_directory("a")
    strata.open_directory("b", directory="a")
    strata.open_directory("c", directory="b")
    strata.open_directory("d", directory="c")
    strata.keyboard.press("End")
    strata.wait_for_focused_entry("199.txt")

    _compress(strata, "199.txt", "199", "d", folder="a/b/c/d")

    strata.wait_for_selection(["199.zip"], directory="d")
    entry = strata.entry("199.zip", directory="d")
    strata.wait(
        lambda: strata.on_screen(entry),
        "the archive to be on screen in the deep column",
    )


def test_columns_reveals_archive_in_parent_column(strata):
    """Columns mode scrolls an archive created in a non-active parent column."""
    root = strata.fixture.root.name
    strata.wait_for_view("Columns")
    strata.open_directory("nested")
    strata.keyboard.press("Left")
    strata.wait_for_focused_entry("nested")
    strata.keyboard.press("End")
    strata.wait_for_focused_entry("199.txt")

    _compress(strata, "199.txt", "199", root)

    strata.wait_for_selection(["199.zip"], directory=root)
    entry = strata.entry("199.zip", directory=root)
    strata.wait(
        lambda: strata.on_screen(entry),
        "the archive to be on screen in the parent column",
    )