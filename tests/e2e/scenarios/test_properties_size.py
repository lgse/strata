# SPDX-License-Identifier: GPL-3.0-or-later
"""Properties measures directory contents rather than directory metadata."""

import pytest

from harness.modes import ALL_MODES


@pytest.fixture
def sized_folder(fixture_tree):
    fixture_tree.populate(
        {
            "sized-folder": {
                "top.txt": "abc",
                "nested": {"child.txt": "12345", ".hidden.txt": "1234567"},
            }
        }
    )
    folder = fixture_tree.path("sized-folder")
    (folder / "nested/loop").symlink_to(folder, target_is_directory=True)
    (folder / "outside").symlink_to(fixture_tree.path("todo.txt"))
    return folder


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("current_folder", [False, True], ids=["entry", "current-folder"])
def test_properties_calculates_nested_and_hidden_file_sizes(
    sized_folder, strata, mode, current_folder
):
    if current_folder:
        strata.open_directory(sized_folder.name)
        strata.pointer.right_click(strata.pane(), at=strata.background_point())
        strata.wait(strata.context_menu, "the folder context menu")
    else:
        strata.open_context_menu(sized_folder.name)
    strata.choose_menu_item("Properties")
    dialog = strata.wait_for_dialog()

    strata.wait(
        lambda: dialog.find(role="label", name="15 B"),
        "Properties to show the recursive size without following symlinks",
    )


@pytest.mark.parametrize("name, expected", [("archive", "0 B"), ("readme.md", "10 B")])
def test_properties_shows_zero_for_empty_folders_and_preserves_file_sizes(
    strata, name, expected
):
    strata.open_context_menu(name)
    strata.choose_menu_item("Properties")
    dialog = strata.wait_for_dialog()

    strata.wait(
        lambda: dialog.find(role="label", name=expected),
        f"Properties to show {expected}",
    )
