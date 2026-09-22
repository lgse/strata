# SPDX-License-Identifier: MIT
"""Properties measures directory contents rather than directory metadata."""

import pytest


def _measurement_finished(dialog):
    spinner = dialog.find(name="Calculating folder size")
    return spinner is None or not spinner.is_rendered()


@pytest.fixture
def sized_folder(fixture_tree):
    fixture_tree.populate(
        {
            "sized-folder": {
                "top.txt": "abc",
                "nested": {"child.txt": "12345", ".hidden.txt": "1234567"},
                ".hidden": {"visible": {"file.txt": "123"}},
            }
        }
    )
    folder = fixture_tree.path("sized-folder")
    (folder / "nested/loop").symlink_to(folder, target_is_directory=True)
    (folder / "outside").symlink_to(fixture_tree.path("todo.txt"))
    return folder


@pytest.mark.parametrize("current_folder", [False, True], ids=["entry", "current-folder"])
def test_properties_calculates_nested_and_hidden_file_sizes(
    sized_folder, strata, current_folder
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
        lambda: dialog.find(role="label", name="18 B"),
        "Properties to show the recursive size without following symlinks",
    )
    strata.wait(lambda: _measurement_finished(dialog), "the size spinner to disappear")
    assert dialog.find(role="label", name="4 files, 1 folder")


def test_properties_explains_unreadable_folder_contents(sized_folder, strata):
    blocked = sized_folder / ".hidden"
    blocked.chmod(0)
    try:
        strata.open_context_menu(sized_folder.name)
        strata.choose_menu_item("Properties")
        dialog = strata.wait_for_dialog()
        strata.wait(lambda: _measurement_finished(dialog), "the measurement to finish")
        assert dialog.find(role="label", name="≥ 15 B")
        assert dialog.find(role="label", name="≥ 4 files, ≥ 1 folder")
        message = "Totals are incomplete.\nSome folders or entries couldn't be read."
        warning = strata.wait(lambda: dialog.find(name=message), "the incomplete measurement warning")
        assert warning.is_rendered()
        strata.pointer.move_to(*warning.screen_bounds().center)
        strata.wait(
            lambda: strata.application.application_node.find(role="label", name=message),
            "the warning tooltip",
        )
    finally:
        blocked.chmod(0o755)


@pytest.mark.parametrize("route", ["keyboard", "context-menu"])
@pytest.mark.parametrize("unreadable", [False, True])
def test_selection_properties_routes_preserve_aggregate_warnings(
    sized_folder, strata, route, unreadable
):
    blocked = sized_folder / ".hidden"
    if unreadable:
        blocked.chmod(0)
    try:
        strata.select_entry("sized-folder")
        strata.pointer.click(strata.entry("readme.md"), modifiers=["ctrl"])
        strata.wait_for_selection(["sized-folder", "readme.md"], sized_folder.parent.name)
        if route == "keyboard":
            strata.keyboard.press("alt+Return")
        else:
            strata.open_context_menu("readme.md", sized_folder.parent.name)
            strata.choose_menu_item("Properties")
        dialog = strata.wait_for_dialog()
        expected = "≥ 25 B" if unreadable else "28 B"
        strata.wait(
            lambda: dialog.find(role="label", name=expected),
            "the combined selection size",
        )
        counts = "≥ 5 files, ≥ 1 folder" if unreadable else "5 files, 1 folder"
        assert dialog.find(role="label", name=counts)
        if unreadable:
            warning = dialog.find(
                name="Totals are incomplete.\nSome folders or entries couldn't be read."
            )
            assert warning and warning.is_rendered()
    finally:
        blocked.chmod(0o755)


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
    strata.wait(lambda: _measurement_finished(dialog), "no spinner after measurement")
