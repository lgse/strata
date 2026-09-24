# SPDX-License-Identifier: MIT
"""Command discovery, focus handoff and file targeting through real input."""

import pytest

from harness.modes import ALL_MODES


def open_palette(strata, query=""):
    strata.keyboard.press("ctrl+shift+p")
    field = strata.wait(
        lambda: strata.window.find(role="text", name="Search commands"),
        "command palette search",
    )
    strata.keyboard.type_text(query)
    strata.wait(lambda: field.text == query, "command query")
    return field


def run_command(strata, query):
    open_palette(strata, query)
    strata.keyboard.press("Return")
    strata.wait(
        lambda: strata.window.find(role="text", name="Search commands") is None,
        "command palette closed",
    )


def test_palette_keyboard_recent_commands_and_settings_handoff(strata):
    strata.select_entry_with_keyboard("todo.txt")
    field = open_palette(strata, "term")
    for direction, character, expected in [("Down", "i", "termi"), ("Up", "n", "termin")]:
        strata.keyboard.press(direction)
        strata.keyboard.type_text(character)
        strata.wait(lambda: field.text == expected, "typing extends the query after navigation")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("Switch to Icons")
    strata.wait(lambda: field.text == "Switch to Icons", "view command query")
    strata.keyboard.press("ctrl+k")
    strata.wait(lambda: field.has_state("focused"), "palette retains keyboard ownership")
    assert strata.window.find(name="Search files and folders…") is None
    for direction in ("Up", "Down"):
        strata.keyboard.press(direction)
        strata.wait(
            lambda: strata.window.find(role="list item", name="Switch to Icons", states={"selected"}),
            "single search result stays selected when wrapping",
        )
    strata.keyboard.press("Return")
    strata.wait_for_view("Icons")
    open_palette(strata)
    strata.wait(
        lambda: strata.window.find(
            role="list item", name="Switch to Icons", description="Recents", states={"selected"}
        ),
        "last command selected in Recents",
    )
    strata.keyboard.press("Up")
    strata.wait(
        lambda: strata.window.find(role="list item", name="Undo last file operation", states={"selected"}),
        "Up wraps from first command to last",
    )
    strata.keyboard.press("Down")
    strata.wait(
        lambda: strata.window.find(role="list item", name="Switch to Icons", states={"selected"}),
        "Down wraps from last command to first",
    )
    strata.keyboard.press("Return")
    strata.wait_for_view("Icons")
    assert strata.window.find(role="text", name="Search commands") is None

    button = strata.window.find(role="button", name="Command palette (Ctrl+Shift+P)")
    assert button is not None and button.activate()
    field = strata.wait(
        lambda: strata.window.find(role="text", name="Search commands"),
        "palette opened from header",
    )
    strata.keyboard.type_text("preferences")
    strata.wait(lambda: field.text == "preferences", "settings alias")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.window.find(role="button", name="Close settings"), "settings handoff")
    assert strata.window.find(role="text", name="Search commands") is None


@pytest.mark.parametrize("mode", ALL_MODES)
def test_palette_rename_targets_filtered_selection(strata, mode):
    strata.select_entry_with_keyboard("readme.md")
    strata.keyboard.press("ctrl+f")
    strata.keyboard.type_text("todo")
    strata.wait(lambda: strata.matches() == ["todo.txt"], "filtered result")
    strata.keyboard.press("Down")
    strata.wait(
        lambda: strata.window.find(name="todo.txt", states={"selected"}),
        "filtered selection",
    )
    open_palette(strata, "rename")
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(name="todo.txt", states={"selected"}),
        "selection restored",
    )
    run_command(strata, "rename")
    editor = strata.wait(
        lambda: strata.window.find(role="text", states={"focused"}),
        "rename editor",
    )
    assert "todo" in editor.text
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("renamed.txt")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.fixture.path("renamed.txt").exists(), "renamed file on disk")
    assert not strata.fixture.path("todo.txt").exists()
    assert strata.fixture.path("readme.md").read_text() == "# Fixture\n"


def test_palette_creates_pins_duplicates_and_undoes(strata):
    run_command(strata, "mkdir")
    strata.wait(
        lambda: strata.window.find(role="text", states={"focused"}),
        "new folder rename editor",
    )
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("palette-folder")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.fixture.path("palette-folder").is_dir(), "new folder")
    strata.wait_for_selection(["palette-folder"], directory=strata.fixture.root.name)
    run_command(strata, "pin folder")
    strata.wait(
        lambda: strata.window.find(role="button", name="palette-folder"),
        "pinned folder in sidebar",
    )
    run_command(strata, "unpin folder")
    strata.wait(
        lambda: strata.window.find(role="button", name="palette-folder") is None,
        "folder unpinned",
    )

    strata.select_entry_with_keyboard("todo.txt")
    before = set(strata.fixture.names())
    run_command(strata, "duplicate")
    copied = strata.wait(lambda: set(strata.fixture.names()) - before, "duplicate created")
    assert len(copied) == 1
    name = copied.pop()
    strata.entry(name)
    assert strata.fixture.path(name).read_text() == "todo\n"
    run_command(strata, "undo")
    strata.wait(lambda: set(strata.fixture.names()) == before, "duplicate undone")
    assert strata.fixture.path("todo.txt").read_text() == "todo\n"


def test_palette_folder_creation_uses_focused_pane_despite_pointer_hover(strata):
    strata.open_directory("documents")
    strata.select_entry_with_keyboard("notes.txt")
    strata.pointer.move_to(*strata.entry("readme.md").screen_bounds().center)
    run_command(strata, "mkdir")
    strata.wait(
        lambda: strata.window.find(role="text", states={"focused"}),
        "new folder editor in focused pane",
    )
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("in-documents")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: strata.fixture.path("documents/in-documents").is_dir(),
        "folder in focused pane",
    )
    assert not strata.fixture.path("in-documents").exists()
