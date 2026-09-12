# SPDX-License-Identifier: MIT
"""Creation keeps the Miller path, selection, and real keyboard focus consistent."""

import pytest


def create_in_parent(strata, kind, stale_child):
    root = strata.fixture.root.name
    if stale_child:
        strata.open_directory("documents")
    strata.pointer.right_click(strata.pane(root), at=strata.background_point(root))
    strata.choose_menu_item("New Folder" if kind == "folder" else "New File")
    field = strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable", "focused"}),
        "the parent rename editor to take keyboard focus",
    )
    original = "new " + kind
    strata.wait(lambda: field.text == original, "the default name")
    path = strata.fixture.path(original)
    assert path.is_dir() if kind == "folder" else path.is_file()
    strata.wait_for_selection([original], root)
    expected = [root, original] if kind == "folder" else [root]
    strata.wait(lambda: strata.pane_names() == expected, "the child path to follow creation")
    assert strata.current_directory() == root
    return field, original


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_created_folder_editor_stays_visible_in_a_narrow_window(strata):
    strata.select_entry("readme.md")
    bounds = strata.window.window_bounds()
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, 420, 300)
    strata.wait(lambda: strata.window.window_bounds().width == 420, "a narrow window")
    strata.keyboard.press("ctrl+shift+n")
    field = strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable", "focused"}),
        "the new folder editor to remain visible in the parent",
    )
    strata.keyboard.type_text("visible-folder")
    strata.wait(lambda: field.text == "visible-folder", "typing in the visible editor")
    editor = field.window_bounds()
    root = strata.pane(strata.fixture.root.name).window_bounds()
    assert editor.width > 1 and editor.x >= max(root.x, 0)
    assert editor.x + editor.width <= strata.window.window_bounds().width
    strata.keyboard.press("Return")
    strata.wait(strata.fixture.path("visible-folder").is_dir, "the visible folder rename")


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
@pytest.mark.parametrize("kind", ["file", "folder"])
@pytest.mark.parametrize("stale_child", [False, True], ids=["no-child", "stale-child"])
@pytest.mark.parametrize("completion", ["enter", "sibling", "navigate"])
def test_created_entry_completion_preserves_the_users_focus(strata, kind, stale_child, completion):
    root = strata.fixture.root.name
    field, original = create_in_parent(strata, kind, stale_child)
    strata.keyboard.type_text("renamed")
    strata.wait(lambda: field.text == "renamed", "typing to replace the entire default name")
    if completion == "enter":
        strata.keyboard.press("Return")
    elif completion == "sibling":
        strata.pointer.click(strata.entry("readme.md", root))
    else:
        strata.pointer.click(strata.sidebar_button("Home"))
    strata.wait(strata.fixture.path("renamed").exists, "the rename on disk")
    strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable"}) is None,
        "the editor to close",
    )
    assert not strata.fixture.path(original).exists()
    if completion == "navigate":
        strata.wait_for_directory(strata.environment.home.name)
        assert root not in strata.pane_names()
    else:
        expected = [root, "renamed"] if kind == "folder" else [root]
        strata.wait(lambda: strata.pane_names() == expected, "the renamed child header")
        selected = "renamed" if completion == "enter" else "readme.md"
        strata.wait_for_selection([selected], root)
        strata.wait_for_directory(root)
        # No pointer correction: F2 must use the item selected by the previous action.
        strata.keyboard.press("F2")
        reopened = strata.wait(
            lambda: strata.window.find(role="text", name="Rename", states={"editable", "focused"}),
            "F2 to rename the still-selected parent item",
        )
        assert reopened.text == selected
        strata.keyboard.press("Escape")
    assert strata.window.find(role="dialog") is None
    assert "Gtk-CRITICAL" not in strata.application.log()
