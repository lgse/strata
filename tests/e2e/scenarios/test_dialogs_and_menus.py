# SPDX-License-Identifier: MIT
"""Context menus, dialogs, Escape handling, and invalid operations."""

from __future__ import annotations

import shlex
import shutil
import time

import pytest

from harness.modes import ALL_MODES, NEXT_ENTRY_KEY, PREVIOUS_ENTRY_KEY
from harness.tree import TreeTimeout

ENTRY_MENU_ITEMS = {"Open", "Cut", "Copy", "Rename", "Move to Trash", "Properties"}


def assert_menu_order(strata, expected):
    items = strata.menu_items()
    positions = [items.index(action) for action in expected]
    assert positions == sorted(positions), f"unexpected action order: {items}"


@pytest.fixture
def executable_file(fixture_tree):
    program = fixture_tree.path("run-me")
    shutil.copy2(shutil.which("true"), program)
    return program


@pytest.fixture
def observable_executable_file(fixture_tree):
    program = fixture_tree.path("run-me")
    marker = fixture_tree.path("run-me.executed")
    program.write_text(f"#!/bin/sh\nprintf executed > {shlex.quote(str(marker))}\n")
    program.chmod(0o755)
    return program


def test_the_entry_context_menu_offers_named_actions_and_accelerators(strata):
    strata.open_context_menu("todo.txt")

    menu = strata.context_menu()
    assert menu is not None, "the context menu should have the menu role"
    items = menu.find_all(role="menu item")
    assert items, "menu entries should have the menu item role"
    assert all(node.name for node in items), "every menu item needs a name"
    assert strata.menu_item("Copy").description == "Ctrl+C", (
        "the accelerator belongs in the description, not the name"
    )
    assert_menu_order(strata, [
        "Open", "Open With…", "Quick preview", "Print", "Cut", "Copy", "Duplicate",
        "Rename", "Move to…", "Copy to…", "Compress…", "Customize…", "Copy path",
        "Copy name", "Properties", "Move to Trash", "Permanently delete",
    ])
    strata.dismiss_menu()

    strata.open_context_menu("documents")
    assert_menu_order(strata, [
        "Open", "Open With…", "Open in Terminal", "Open in…", "Cut", "Copy", "Duplicate",
        "Rename", "Move to…", "Copy to…", "Compress…", "Pin to sidebar",
        "Customize…", "Copy path", "Copy name", "Properties", "Move to Trash",
        "Permanently delete",
    ])
    strata.dismiss_menu()


@pytest.mark.preferences(browser_mode="list", single_click_previews=False)
def test_secondary_click_retargets_an_open_context_menu(strata):
    root = strata.fixture.root.name
    strata.select_entry("todo.txt", root)
    strata.click_entry_with("readme.md", ["ctrl"], directory=root)
    strata.wait_for_selection(["readme.md", "todo.txt"], root)

    strata.pointer.right_click(strata.entry("todo.txt", root))
    strata.wait(strata.context_menu, "the grouped context menu")
    assert "Rename" not in strata.menu_items()

    strata.pointer.right_click(strata.entry("archive", root))
    strata.wait_for_selection(["archive"], root)
    strata.wait(lambda: "Rename" in strata.menu_items(), "the retargeted item menu")
    strata.dismiss_menu()


@pytest.fixture
def context_actions(test_environment):
    applications = test_environment.data_home / "applications"
    applications.mkdir()
    (applications / "strata-context-viewer.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Context Viewer\n"
        "Exec=true %U\nMimeType=text/plain;text/markdown;\n"
    )
    (test_environment.config_home / "mimeapps.list").write_text(
        "[Default Applications]\n"
        "text/plain=strata-context-viewer.desktop;\n"
        "text/markdown=strata-context-viewer.desktop;\n"
        "[Added Associations]\n"
        "text/plain=strata-context-viewer.desktop;\n"
        "text/markdown=strata-context-viewer.desktop;\n"
    )
    for name in ("First action", "Second action"):
        action_id = name.lower().replace(" ", "-")
        directory = test_environment.config_home / "strata/actions" / action_id
        directory.mkdir(parents=True)
        (directory / "action.toml").write_text(f'''schema_version = 1
id = "{action_id}"
name = "{name}"
menu = "submenu"
[when]
[run]
runtime = "command"
program = "true"
args = ["{{paths}}"]
''')


@pytest.mark.usefixtures("context_actions")
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("shortcut,activation", [("Menu", "Return"), ("shift+F10", "space")])
@pytest.mark.preferences(show_hidden=False, single_click_previews=False)
def test_keyboard_context_menu_targets_selection_and_owns_keys(strata, mode, shortcut, activation):
    root = strata.fixture.root.name
    before = strata.fixture.listing()
    strata.select_entry("todo.txt", root)
    strata.wait_for_focused_entry("todo.txt")
    strata.keyboard.press(shortcut)
    strata.wait(strata.context_menu, "the keyboard item menu")
    assert ENTRY_MENU_ITEMS <= set(strata.menu_items())
    assert "New Folder" not in strata.menu_items()
    strata.wait(
        lambda: "focused" in strata.menu_item("New Folder with Selection").states,
        "the first item to receive focus",
    )

    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.context_menu() is None, "Escape to dismiss the menu")
    strata.wait_for_selection(["todo.txt"], root)
    strata.wait_for_focused_entry("todo.txt")
    assert strata.fixture.listing() == before

    strata.click_entry_with("readme.md", ["ctrl"], directory=root)
    strata.wait_for_selection(["readme.md", "todo.txt"], root)
    strata.keyboard.press(shortcut)
    strata.wait(strata.context_menu, "the multi-selection menu")
    assert "Rename" not in strata.menu_items()
    assert "Actions" in strata.menu_items()
    assert_menu_order(strata, [
        "Open With…", "Cut", "Copy", "Duplicate", "Move to…", "Copy to…",
        "Compress…", "Copy paths", "Copy names", "Properties", "Move to Trash",
        "Permanently delete",
    ])
    strata.wait(
        lambda: strata.menu_item("New Folder with Selection").has_state("focused"),
        "New Folder with Selection to receive initial focus with multiple files and custom actions",
    )
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


@pytest.mark.usefixtures("context_actions")
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.preferences(single_click_previews=False)
def test_submenu_arrows_return_keyboard_control_to_the_parent(strata, mode):
    root = strata.fixture.root.name
    strata.select_entry("todo.txt", root)
    strata.click_entry_with("readme.md", ["ctrl"], directory=root)
    strata.wait_for_selection(["readme.md", "todo.txt"], root)
    strata.keyboard.press("shift+F10")
    strata.wait(strata.context_menu, "multi-selection context menu")
    strata.keyboard.press("Home")
    for _ in strata.menu_items():
        if strata.menu_item("Actions").has_state("focused"):
            break
        strata.keyboard.press("Down")
    strata.wait(lambda: strata.menu_item("Actions").has_state("focused"), "Actions focus")

    for opener in ("Right", "space", "Return"):
        strata.keyboard.press(opener)
        strata.wait(
            lambda: strata.menu_item("First action").has_state("focused"),
            "first submenu action focus",
        )
        strata.keyboard.press("Down")
        strata.wait(
            lambda: strata.menu_item("Second action").has_state("focused"),
            "navigation inside the submenu",
        )
        strata.keyboard.press("Left")
        strata.wait(
            lambda: strata.window.find(role="menu item", name="Second action") is None,
            "Left to close only the submenu",
        )
        strata.wait(lambda: strata.menu_item("Actions").has_state("focused"), "owner focus")
        strata.keyboard.press("Down")
        strata.wait(lambda: strata.menu_item("Cut").has_state("focused"), "parent Down navigation")
        strata.keyboard.press("Up")
        strata.wait(lambda: strata.menu_item("Actions").has_state("focused"), "parent Up navigation")
        strata.wait_for_selection(["readme.md", "todo.txt"], root)

    strata.keyboard.press("Down")
    strata.keyboard.press("Down")
    strata.wait(lambda: strata.menu_item("Copy").has_state("focused"), "Copy focus after submenu return")
    strata.keyboard.press("Return")
    strata.wait_for_menu_closed()
    strata.open_directory("archive")
    strata.paste_into("archive")
    for name in ("readme.md", "todo.txt"):
        copied = strata.fixture.path(f"archive/{name}")
        strata.wait(copied.exists, f"{name} copied from the parent menu")
        assert copied.read_bytes() == strata.fixture.path(name).read_bytes()


def test_the_pane_context_menu_offers_directory_actions(strata):
    strata.pointer.right_click(strata.pane(), at=strata.background_point())
    strata.wait(strata.context_menu, "the pane context menu")

    offered = set(strata.menu_items())
    assert {"New Folder", "Select All", "Refresh"} <= offered, (
        f"unexpected pane menu {sorted(offered)}"
    )
    assert_menu_order(strata, [
        "New Folder", "New File", "Paste", "Open With…", "Open in Terminal",
        "Select All", "Refresh", "Customize…", "Properties",
    ])
    strata.dismiss_menu()


def test_folder_background_customize_targets_the_presented_directory(strata):
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
@pytest.mark.usefixtures("unreserved_columns")
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
        lambda: "focused" in strata.dialog_button("Run").states,
        "Run to receive initial focus",
    )
    assert strata.dialog_button("Close dialog").activate()
    strata.wait(lambda: strata.dialog() is None, "the close button to dismiss the dialog")

    strata.double_click_entry(executable_file.name)
    strata.pointer.click(strata.dialog_button("Run"))
    strata.wait(lambda: strata.dialog() is None, "the confirmed program to launch")


def test_executable_context_menu_offers_confirmed_run(observable_executable_file, strata):
    marker = observable_executable_file.with_name("run-me.executed")
    strata.open_context_menu(observable_executable_file.name)
    assert {"Open", "Open With…", "Run"} <= set(strata.menu_items())
    strata.choose_menu_item("Run")

    dialog = strata.wait_for_dialog()
    assert "Run this program?" in dialog.dump()
    strata.pointer.click(strata.dialog_button("Cancel"))
    strata.wait(lambda: strata.dialog() is None, "the cancelled run dialog to close")
    assert not marker.exists(), "Cancel must not launch the program"

    strata.open_context_menu(observable_executable_file.name)
    strata.choose_menu_item("Run")
    strata.pointer.click(strata.dialog_button("Run"))
    strata.wait(lambda: strata.dialog() is None, "the confirmed program to launch")
    strata.wait(marker.exists, "the confirmed program to create its marker")


def test_properties_pins_a_folder_and_offers_unpin_afterwards(strata):
    dialog = _open_properties(strata, "documents")
    pin = dialog.find(role="button", name="Pin")
    assert pin is not None, dialog.dump()
    assert "sensitive" in pin.states
    assert pin.activate()
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

    assert unpin.activate()
    strata.wait(lambda: strata.dialog() is None, "the dialog to close after unpinning")
    strata.wait(
        lambda: strata.window.find(role="button", name="documents") is None,
        "the sidebar row to disappear",
    )


@pytest.mark.preferences(browser_mode="columns")
@pytest.mark.parametrize("opener,dismissal", [
    ("keyboard-menu", "Escape"),
    ("pointer-menu", "Close dialog"),
    ("shortcut", "backdrop"),
    ("pointer-menu", "Rename"),
])
def test_file_properties_describes_the_file_without_pin_actions_and_closes(
    strata, opener, dismissal,
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
    strata.keyboard.press("Up")
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

    dialog = strata.wait_for_dialog()
    assert dialog.name == "Unable to rename item"
    assert fixture.path("todo.txt").exists(), "the rename must not silently succeed"
    assert fixture.path("readme.md").read_text() == "# Fixture\n", (
        "the existing file must keep its contents"
    )
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "the rename error to close")
    strata.wait_for_focused_entry("todo.txt")
    strata.keyboard.press("Up")
    strata.wait_for_focused_entry("readme.md")


def _describe_focus(strata) -> str:
    focused = strata.focused_node()
    if focused is None:
        return "nothing"
    return f"{focused.role} {focused.name!r}"


def _focus_returned(strata, name: str, failures: list[str], step: str) -> bool:
    try:
        strata.wait(lambda: strata.focused_name() == name, f"focus to return to {name!r}", timeout=4)
        return True
    except TreeTimeout:
        failures.append(f"{step}: focus is on {_describe_focus(strata)}")
        strata.select_entry(name)
        strata.wait_for_focused_entry(name)
        return False


def _open_settings_and_close(strata, close):
    strata.keyboard.press("ctrl+,")
    strata.wait(
        lambda: strata.window.find(role="button", name="Close settings"), "Settings to open"
    )
    if close == "Escape":
        strata.keyboard.press("Escape")
    else:
        assert strata.window.find(role="button", name="Close settings").activate()
    strata.wait(
        lambda: strata.window.find(role="button", name="Close settings") is None,
        "Settings to close",
    )


def _open_palette_and_close(strata, opener, close):
    def fields():
        return len(strata.window.find_all(role="text", states={"editable"}))

    closed = fields()
    strata.keyboard.press(opener)
    strata.editable_field()
    strata.keyboard.press(close)
    strata.wait(lambda: fields() == closed, "the palette to close")


def _open_compress_and_close(strata, close):
    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Compress…")
    strata.wait_for_dialog()
    strata.editable_field()
    if close == "Escape":
        strata.keyboard.press("Escape")
    elif close == "backdrop":
        bounds = strata.window.screen_bounds()
        strata.pointer.click(strata.window, at=(bounds.x + 5, bounds.y + 5))
    else:
        strata.pointer.click(strata.dialog_button(close))
    strata.wait(lambda: strata.dialog() is None, "Compress to close")


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.preferences(single_click_previews=False)
def test_dismissed_overlays_return_focus_to_the_file_list(strata, mode):
    steps = [
        ("Settings, Escape", lambda: _open_settings_and_close(strata, "Escape")),
        ("Settings, Close settings", lambda: _open_settings_and_close(strata, "Close settings")),
        ("Ctrl+K, Escape", lambda: _open_palette_and_close(strata, "ctrl+k", "Escape")),
        ("Ctrl+K, Ctrl+K", lambda: _open_palette_and_close(strata, "ctrl+k", "ctrl+k")),
        ("Ctrl+Shift+K, Escape", lambda: _open_palette_and_close(strata, "ctrl+shift+k", "Escape")),
        ("Compress, Escape", lambda: _open_compress_and_close(strata, "Escape")),
        ("Compress, Cancel", lambda: _open_compress_and_close(strata, "Cancel")),
        ("Compress, backdrop", lambda: _open_compress_and_close(strata, "backdrop")),
    ]
    failures: list[str] = []
    strata.select_entry("todo.txt")
    strata.wait_for_focused_entry("todo.txt")
    for step, run in steps:
        run()
        if not _focus_returned(strata, "todo.txt", failures, step):
            continue
        strata.keyboard.press(PREVIOUS_ENTRY_KEY[mode])
        strata.wait_for_focused_entry("readme.md")
        strata.keyboard.press(NEXT_ENTRY_KEY[mode])
        strata.wait_for_focused_entry("todo.txt")
    assert not failures, f"{mode}: dismissed overlays left focus elsewhere:\n" + "\n".join(failures)


def _open_customize(strata, opener):
    if opener == "item-menu":
        strata.open_context_menu("todo.txt")
    else:
        root = strata.fixture.root.name
        strata.pointer.right_click(strata.pane(root), at=strata.background_point(root))
        strata.wait(strata.context_menu, "the pane context menu")
    strata.choose_menu_item("Customize…")
    return strata.wait_for_dialog()


@pytest.mark.parametrize("opener", ["item-menu", "background-menu"])
@pytest.mark.preferences(single_click_previews=False)
def test_customize_takes_focus_and_a_single_escape_closes_it(strata, opener):
    failures: list[str] = []
    strata.select_entry("todo.txt")
    strata.wait_for_focused_entry("todo.txt")
    _open_customize(strata, opener)
    try:
        strata.wait(
            lambda: strata.dialog_button("Done").has_state("focused"), "Done to take focus", timeout=4
        )
    except TreeTimeout:
        failures.append(f"on open: focus is on {_describe_focus(strata)}, not Done")
    strata.keyboard.press("Escape")
    try:
        strata.wait(lambda: strata.dialog() is None, "one Escape to close Customize", timeout=4)
    except TreeTimeout:
        failures.append("the first Escape left Customize open")
        strata.keyboard.press("Escape")
        strata.wait(lambda: strata.dialog() is None, "a second Escape to close Customize")
    if opener == "item-menu":
        if _focus_returned(strata, "todo.txt", failures, "after Escape"):
            strata.keyboard.press("Up")
            strata.wait_for_focused_entry("readme.md")
            strata.keyboard.press("Down")
    else:
        try:
            strata.wait(lambda: strata.focused_pane() is not None, "focus to return to the pane", timeout=4)
        except TreeTimeout:
            failures.append(f"after Escape: focus is on {_describe_focus(strata)}, not the pane")

    if opener == "item-menu":
        strata.wait_for_focused_entry("todo.txt")
        dialog = _open_customize(strata, opener)
        custom = dialog.find(role="button", name="Custom color…")
        if custom is None:
            failures.append("the custom color button has no accessible name")
        else:
            def custom_color_dialog():
                return strata.window.find(role="dialog", name="Custom File Color")

            strata.pointer.click(custom)
            nested = strata.wait(custom_color_dialog, "the custom color dialog")
            strata.pointer.click(nested.find(role="button", name="Cancel"))
            strata.wait(
                lambda: custom_color_dialog() is None and strata.dialog() is not None,
                "Customize to remain open",
            )
            focused = strata.focused_node()
            if focused is None or focused.name != "Custom color…":
                failures.append(f"after Cancel: focus is on {_describe_focus(strata)}")
        strata.keyboard.press("Escape")
        try:
            strata.wait(lambda: strata.dialog() is None, "one Escape to close Customize", timeout=4)
        except TreeTimeout:
            failures.append("reopened: the first Escape left Customize open")
            strata.keyboard.press("Escape")
            strata.wait(lambda: strata.dialog() is None, "a second Escape to close Customize")
        _focus_returned(strata, "todo.txt", failures, "reopened, after Escape")
    assert not failures, f"{opener}:\n" + "\n".join(failures)


def test_the_shortcut_reference_opens_and_closes(strata):
    strata.keyboard.press("F1")

    strata.wait(
        lambda: strata.window.find(role="label", name="Keyboard shortcuts"),
        "the shortcut reference to open",
    )
    for chord in ["Ctrl+Alt+Space", "Ctrl+Alt+← / →", "Ctrl+Alt+↑ / ↓", "Ctrl+Alt+M"]:
        assert strata.window.find(role="label", name=chord, rendered=False) is not None
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


def test_enter_submits_compress_then_floating_chooser_extracts(strata):
    destination = strata.fixture.path("unpacked")
    destination.mkdir()
    strata.entry("unpacked")
    compress_from_the_context_menu(strata, "readme.md", "bundle")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.dialog() is None, "the compress dialog to close")
    strata.wait(
        lambda: strata.fixture.path("bundle.zip").exists(), "the archive to be created"
    )

    strata.open_context_menu("bundle.zip")
    assert_menu_order(strata, [
        "Open", "Open With…", "Extract here", "Extract to…", "Cut", "Copy",
        "Duplicate", "Rename", "Move to…", "Copy to…", "Compress…",
        "Customize…", "Copy path", "Copy name", "Properties", "Move to Trash",
        "Permanently delete",
    ])
    strata.choose_menu_item("Extract to…")
    chooser = strata.destination_chooser("Extract to")
    strata.navigate_destination(chooser, destination)
    strata.confirm_destination(chooser, "Extract here")

    extracted = destination / "readme.md"
    # Extraction creates each member before streaming its bytes into place.
    strata.wait(
        lambda: extracted.is_file() and extracted.read_text() == "# Fixture\n",
        "the chooser action to extract the complete member into the destination",
    )


@pytest.mark.parametrize("action,accept_label", [("Copy to…", "Copy here"), ("Move to…", "Move here")])
def test_floating_destination_chooser_transfers_files(strata, action, accept_label):
    destination = strata.fixture.path("documents")
    source = strata.fixture.path("todo.txt")
    strata.open_context_menu("todo.txt")
    strata.choose_menu_item(action)
    chooser = strata.destination_chooser(action.rstrip("…"))
    strata.navigate_destination(chooser, destination)
    assert strata.current_directory() == strata.fixture.root.name
    strata.confirm_destination(chooser, accept_label)
    strata.wait(lambda: (destination / "todo.txt").exists(), "transfer into the chosen destination")
    assert source.exists() == (action == "Copy to…")


@pytest.mark.preferences(browser_mode="list")
def test_list_transfer_reveal_into_a_visited_folder_selects_the_copy(strata):
    destination = strata.fixture.path("documents")
    strata.open_directory("documents")
    strata.wait_for_focused_entry("notes.txt")
    strata.keyboard.press("Down")
    strata.wait_for_focused_entry("report.md")
    strata.keyboard.press("alt+Left")
    strata.wait_for_directory(strata.fixture.root.name)
    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Copy to…")
    chooser = strata.destination_chooser("Copy to")
    strata.navigate_destination(chooser, destination)
    strata.confirm_destination(chooser, "Copy here")
    strata.wait(lambda: (destination / "todo.txt").exists(), "transfer into the chosen destination")
    strata.wait_for_directory("documents")
    strata.wait_for_selection(["todo.txt"])


@pytest.mark.preferences(arrow_navigation_scoped=True, single_click_previews=False)
def test_window_shortcuts_stay_blocked_over_a_modal_dialog(strata):
    default = "Permanently delete 1 item"
    strata.select_entry("todo.txt")
    strata.wait_for_focused_entry("todo.txt")
    strata.keyboard.press("shift+Delete")
    strata.wait_for_dialog()

    def focused():
        return strata.dialog_button(default).has_state("focused")

    strata.wait(focused, "the default button to take focus")
    for chord in ["ctrl+k", "ctrl+shift+k", "ctrl+\\"]:
        strata.keyboard.press(chord)
        # A wrongly routed accelerator acts within this window.
        time.sleep(0.5)
        assert strata.dialog() is not None, chord
        assert strata.window.find(role="text", states={"editable"}) is None, f"{chord} opened a palette"
        assert focused(), f"{chord}: focus is on {_describe_focus(strata)}"
        assert strata.environment.read_preferences().get("arrow_navigation_scoped") == "true", chord

    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "Escape to close the dialog")
    strata.wait_for_focused_entry("todo.txt")
    assert strata.window.find(role="text", states={"editable"}) is None
    assert strata.fixture.path("todo.txt").exists()


def _settings_has_key(strata, relative):
    try:
        text = strata.environment.settings_path.read_text()
    except OSError:
        return False
    return f'"{strata.fixture.path(relative)}"' in text


def _customize_red_code(strata, name, directory=None):
    strata.open_context_menu(name, directory=directory)
    strata.choose_menu_item("Customize…")
    dialog = strata.wait_for_dialog()
    red = dialog.find(role="button", name="Red")
    assert red is not None, f"no Red color button\n{dialog.dump()}"
    strata.pointer.click(red)
    strata.pointer.click(strata.dialog_button("Code"))
    strata.pointer.click(strata.dialog_button("Done"))
    strata.wait(lambda: strata.dialog() is None, "the customize dialog to close")
    strata.wait(
        lambda: _settings_has_key(strata, name if directory is None else f"{directory}/{name}"),
        f"the {name!r} customization to be saved",
    )


def test_customization_follows_rename_and_a_new_folder_at_the_old_path_is_plain(strata):
    _customize_red_code(strata, "documents")

    strata.open_context_menu("documents")
    strata.choose_menu_item("Rename")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("docs2")
    strata.wait(lambda: field.text == "docs2", "the new name to be typed")
    strata.keyboard.press("Return")
    strata.entry("docs2")

    strata.wait(
        lambda: _settings_has_key(strata, "docs2") and not _settings_has_key(strata, "documents"),
        "the customization to follow the rename to docs2",
    )

    strata.fixture.path("documents").mkdir()
    strata.entry("documents")
    strata.open_context_menu("documents")
    strata.choose_menu_item("Customize…")
    strata.wait_for_dialog()
    assert not strata.dialog_button("Clear").has_state("sensitive"), (
        "a new folder at the old path inherited the customization"
    )
    strata.pointer.click(strata.dialog_button("Done"))
    strata.wait(lambda: strata.dialog() is None, "the customize dialog to close")


# Columns would open `pictures` beside the root and scroll `archive` under the sidebar.
@pytest.mark.preferences(browser_mode="list")
def test_customization_follows_a_cut_and_paste_move_and_leaves_with_trash(strata):
    _customize_red_code(strata, "pictures")

    strata.select_entry("pictures")
    strata.keyboard.press("ctrl+x")
    strata.open_directory("archive")
    strata.paste_into("archive")
    strata.wait(
        lambda: strata.fixture.path("archive/pictures").is_dir()
        and not strata.fixture.path("pictures").exists(),
        "pictures to move into archive",
    )
    strata.wait(
        lambda: _settings_has_key(strata, "archive/pictures")
        and not _settings_has_key(strata, "pictures"),
        "the customization to follow the move into archive",
    )

    strata.select_entry("pictures", directory="archive")
    strata.keyboard.press("Delete")
    strata.wait(
        lambda: not strata.fixture.path("archive/pictures").exists(), "pictures to be trashed"
    )
    strata.wait(
        lambda: not _settings_has_key(strata, "archive/pictures"),
        "the customization to leave settings.toml with the trashed folder",
    )
    strata.keyboard.press("ctrl+z")
    strata.wait(
        lambda: strata.fixture.path("archive/pictures").is_dir(), "pictures to be restored"
    )
    strata.wait(
        lambda: _settings_has_key(strata, "archive/pictures"),
        "the customization to come back with the restored folder",
    )
