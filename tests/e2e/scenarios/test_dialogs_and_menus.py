# SPDX-License-Identifier: MIT
"""Context menus, dialogs, Escape handling, and invalid operations."""

from __future__ import annotations

import shutil

import pytest

from harness.modes import ALL_MODES

ENTRY_MENU_ITEMS = {"Open", "Cut", "Copy", "Rename", "Move to Trash", "Properties"}


@pytest.fixture
def executable_file(fixture_tree):
    program = fixture_tree.path("run-me")
    shutil.copy2(shutil.which("true"), program)
    return program


@pytest.mark.parametrize("mode", ALL_MODES)
def test_the_entry_context_menu_offers_named_actions_and_accelerators(strata, mode):
    strata.open_context_menu("todo.txt")

    menu = strata.context_menu()
    assert menu is not None, "the context menu should have the menu role"
    items = menu.find_all(role="menu item")
    assert items, "menu entries should have the menu item role"
    assert all(node.name for node in items), "every menu item needs a name"
    assert strata.menu_item("Copy").description == "Ctrl+C", (
        "the accelerator belongs in the description, not the name"
    )
    offered = set(strata.menu_items())
    assert ENTRY_MENU_ITEMS <= offered, (
        f"missing {sorted(ENTRY_MENU_ITEMS - offered)} from {sorted(offered)}"
    )
    strata.dismiss_menu()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("shortcut,activation", [("Menu", "Return"), ("shift+F10", "space")])
@pytest.mark.preferences(show_hidden=False, single_click_previews=False)
def test_keyboard_context_menu_targets_selection_and_owns_keys(strata, mode, shortcut, activation):
    root = strata.fixture.root.name
    strata.select_entry("todo.txt", root)
    strata.wait_for_focused_entry("todo.txt")
    strata.keyboard.press(shortcut)
    strata.wait(strata.context_menu, "the keyboard item menu")
    assert ENTRY_MENU_ITEMS <= set(strata.menu_items())
    assert "New Folder" not in strata.menu_items()

    strata.keyboard.press("Home")
    strata.wait(lambda: "focused" in strata.menu_item("Open").states, "Home to focus Open")
    strata.keyboard.press("Up")
    strata.wait(
        lambda: "focused" in strata.menu_item("Permanently delete").states,
        "Up to wrap to the last action",
    )
    strata.keyboard.press("Down")
    strata.wait(lambda: "focused" in strata.menu_item("Open").states, "Down to wrap to Open")
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.context_menu() is None, "Escape to dismiss the menu")
    strata.wait_for_selection(["todo.txt"], root)
    strata.wait_for_focused_entry("todo.txt")

    strata.click_entry_with("readme.md", ["ctrl"], directory=root)
    strata.wait_for_selection(["readme.md", "todo.txt"], root)
    strata.keyboard.press(shortcut)
    strata.wait(strata.context_menu, "the multi-selection menu")
    assert "Rename" not in strata.menu_items()
    strata.keyboard.press("ctrl+a")
    assert strata.context_menu() is not None
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.context_menu() is None, "the multi-selection menu to close")
    strata.wait_for_selection(["readme.md", "todo.txt"], root)

    strata.keyboard.press("Escape")
    strata.wait_for_selection([], root)
    strata.keyboard.press(shortcut)
    strata.wait(strata.context_menu, "the unselected pane menu")
    assert "New Folder" in strata.menu_items()
    strata.keyboard.press("Home")
    for _ in range(30):
        if "focused" in strata.menu_item("Select All").states:
            break
        strata.keyboard.press("Down")
    assert "focused" in strata.menu_item("Select All").states
    strata.keyboard.press(activation)
    strata.wait(lambda: strata.context_menu() is None, f"{activation} to activate Select All")
    strata.wait_for_selection([entry.name for entry in strata.entries(root)], root)


def test_escape_closes_the_context_menu_without_acting(strata):
    before = strata.fixture.listing()
    strata.open_context_menu("todo.txt")

    strata.keyboard.press("Escape")

    strata.wait(lambda: strata.context_menu() is None, "the menu to close")
    assert strata.fixture.listing() == before


def test_the_pane_context_menu_offers_directory_actions(strata):
    strata.pointer.right_click(strata.pane(), at=strata.background_point())
    strata.wait(strata.context_menu, "the pane context menu")

    offered = set(strata.menu_items())
    assert {"New Folder", "Select All", "Refresh"} <= offered, (
        f"unexpected pane menu {sorted(offered)}"
    )
    strata.dismiss_menu()


@pytest.mark.parametrize("mode", ALL_MODES)
def test_folder_background_customize_targets_the_presented_directory(strata, mode):
    root = strata.fixture.root.name
    strata.pointer.right_click(strata.pane(root), at=strata.background_point(root))
    strata.wait(strata.context_menu, "the pane context menu")
    strata.choose_menu_item("Customize…")

    dialog = strata.wait_for_dialog()
    assert "Customize Folder" in dialog.dump()
    assert root in dialog.dump()
    strata.pointer.click(strata.dialog_button("Done"))
    strata.wait(lambda: strata.dialog() is None, "the customize dialog to close")


@pytest.mark.preferences(browser_mode="columns")
def test_folder_background_customize_targets_a_non_active_ancestor_column(strata):
    nested = strata.fixture.path("documents/nested")
    nested.mkdir()
    (nested / "child.txt").write_text("child")
    strata.open_directory("documents")
    strata.open_directory("nested", directory="documents")

    strata.pointer.right_click(
        strata.pane("documents"), at=strata.background_point("documents")
    )
    strata.wait(strata.context_menu, "the ancestor pane context menu")
    strata.choose_menu_item("Customize…")

    dialog = strata.wait_for_dialog()
    contents = dialog.dump()
    assert "Customize Folder" in contents
    assert "documents" in contents
    assert "nested" not in contents
    strata.pointer.click(strata.dialog_button("Done"))
    strata.wait(lambda: strata.dialog() is None, "the customize dialog to close")


def _open_properties(strata, name, directory=None):
    strata.open_context_menu(name, directory=directory)
    strata.choose_menu_item("Properties")
    return strata.wait_for_dialog()


def test_executable_without_handler_requires_confirmation(executable_file, strata):
    strata.double_click_entry(executable_file.name)

    dialog = strata.wait_for_dialog()
    assert "Run this program?" in dialog.dump()
    strata.wait(
        lambda: "focused" in strata.dialog_button("Cancel").states,
        "Cancel to receive initial focus",
    )
    assert strata.dialog_button("Close dialog").activate()
    strata.wait(lambda: strata.dialog() is None, "the close button to dismiss the dialog")

    strata.double_click_entry(executable_file.name)
    strata.pointer.click(strata.dialog_button("Run"))
    strata.wait(lambda: strata.dialog() is None, "the confirmed program to launch")


def test_properties_pins_a_folder_and_offers_unpin_afterwards(strata):
    dialog = _open_properties(strata, "documents")
    pin = dialog.find(role="button", name="Pin")
    assert pin is not None, dialog.dump()
    assert "sensitive" in pin.states
    strata.pointer.click(pin)
    strata.wait(lambda: strata.dialog() is None, "the dialog to close after pinning")
    strata.wait(
        lambda: strata.window.find(role="button", name="documents"),
        "the pinned sidebar row",
    )

    dialog = _open_properties(strata, "documents")
    unpin = dialog.find(role="button", name="Unpin")
    assert unpin is not None, (
        f"Properties must offer Unpin for a pinned folder\n{dialog.dump()}"
    )
    assert "sensitive" in unpin.states, "the Unpin control must stay readable"
    assert dialog.find(role="button", name="Pin") is None

    strata.pointer.click(unpin)
    strata.wait(lambda: strata.dialog() is None, "the dialog to close after unpinning")
    strata.wait(
        lambda: strata.window.find(role="button", name="documents") is None,
        "the sidebar row to disappear",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("opener,dismissal", [
    ("keyboard-menu", "Escape"),
    ("pointer-menu", "Close dialog"),
    ("shortcut", "backdrop"),
    ("pointer-menu", "Rename"),
])
def test_file_properties_describes_the_file_without_pin_actions_and_closes(
    strata, mode, opener, dismissal,
):
    strata.fixture.path("documents/readme.md").write_text("Nested fixture\n")
    strata.open_directory("documents")
    strata.select_entry("readme.md", "documents")
    strata.wait_for_focused_entry("readme.md")
    if opener == "keyboard-menu":
        strata.keyboard.press("shift+F10")
        strata.wait(strata.context_menu, "the child-column item menu")
        strata.keyboard.press("Home")
        for _ in range(30):
            if strata.menu_item("Properties").has_state("focused"):
                break
            strata.keyboard.press("Down")
        assert strata.menu_item("Properties").has_state("focused")
        strata.keyboard.press("Return")
        dialog = strata.wait_for_dialog()
    elif opener == "shortcut":
        strata.keyboard.press("alt+Return")
        dialog = strata.wait_for_dialog()
    else:
        dialog = _open_properties(strata, "readme.md", "documents")
    assert "documents/readme.md" in dialog.dump(), "the dialog should describe the child file"
    assert dialog.find(role="button", name="Pin") is None, dialog.dump()
    assert dialog.find(role="button", name="Unpin") is None, dialog.dump()

    if dismissal == "Escape":
        strata.keyboard.press("Escape")
    elif dismissal == "backdrop":
        bounds = strata.window.screen_bounds()
        strata.pointer.click(strata.window, at=(bounds.x + 5, bounds.y + 5))
    else:
        strata.pointer.click(strata.dialog_button(dismissal))
    strata.wait(lambda: strata.dialog() is None, "Properties to close")
    if dismissal == "Rename":
        field = strata.editable_field()
        assert field.text == "readme.md", "Properties must hand focus to the rename editor"
        strata.keyboard.press("Escape")
    strata.wait_for_focused_entry("readme.md")
    strata.wait_for_selection(["readme.md"], "documents")
    strata.keyboard.press("Left" if mode == "Icons" else "Up")
    strata.wait_for_focused_entry("notes.txt")
    strata.wait_for_selection(["notes.txt"], "documents")
    assert strata.fixture.path("documents/readme.md").read_text() == "Nested fixture\n"
    assert strata.fixture.path("readme.md").read_text() == "# Fixture\n"


def test_renaming_onto_an_existing_name_is_rejected(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("readme.md")
    strata.keyboard.press("Return")

    assert fixture.path("todo.txt").exists(), "the rename must not silently succeed"
    assert fixture.path("readme.md").read_text() == "# Fixture\n", (
        "the existing file must keep its contents"
    )
    strata.keyboard.press("Escape")


def test_the_shortcut_reference_opens_and_closes(strata):
    strata.keyboard.press("F1")

    strata.wait(
        lambda: strata.window.find(role="label", name="Keyboard shortcuts"),
        "the shortcut reference to open",
    )

    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="label", name="Keyboard shortcuts") is None,
        "Escape to close the shortcut reference",
    )


def compress_from_the_context_menu(strata, entry_name, archive_name):
    strata.open_context_menu(entry_name)
    strata.choose_menu_item("Compress…")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(archive_name)
    strata.wait(
        lambda: field.text == archive_name, f"{archive_name!r} to reach the name field"
    )
    return field


def test_an_invalid_archive_name_keeps_the_compress_dialog_open(strata):
    compress_from_the_context_menu(strata, "readme.md", "../escape")

    strata.keyboard.press("Return")

    dialog = strata.wait_for_dialog()
    assert dialog.name == "Compress 1 item", (
        f"an invalid name must keep the dialog open, got {dialog.name!r}"
    )
    assert not strata.fixture.path("escape.zip").exists(), (
        "an invalid name must not produce an archive"
    )
    strata.keyboard.press("Escape")


def test_enter_submits_compress_and_extract_to_dialogs(strata):
    compress_from_the_context_menu(strata, "readme.md", "bundle")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.dialog() is None, "the compress dialog to close")
    strata.wait(
        lambda: strata.fixture.path("bundle.zip").exists(), "the archive to be created"
    )

    destination = strata.fixture.path("unpacked")
    strata.open_context_menu("bundle.zip")
    strata.choose_menu_item("Extract to…")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(destination))
    strata.wait(
        lambda: field.text == str(destination), "the destination to reach the field"
    )

    strata.keyboard.press("Return")

    strata.wait(
        lambda: (destination / "readme.md").exists(),
        "Enter to extract into the destination",
    )
    assert (destination / "readme.md").read_text() == "# Fixture\n"


def test_enter_submits_the_copy_to_dialog(strata):
    destination = strata.fixture.path("documents")

    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Copy to…")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(destination))
    strata.wait(
        lambda: field.text == str(destination), "the destination to reach the field"
    )

    strata.keyboard.press("Return")

    strata.wait(
        lambda: (destination / "todo.txt").exists(),
        "Enter to copy into the destination",
    )
