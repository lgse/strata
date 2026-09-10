# SPDX-License-Identifier: MIT
"""Immediate creation and consistent file/folder rename finalization."""

import pytest

from harness.artifacts import ArtifactCollector
from harness.modes import ALL_MODES
from harness.screenshots import capture
from harness.tree import Atspi

KINDS = ["file", "folder"]


@pytest.mark.parametrize("mode", ALL_MODES)
def test_long_rename_keeps_caret_visible(strata, mode, request):
    name = "synthetic-quarterly-report-with-a-very-long-descriptive-basename-2026.txt"
    strata.fixture.path(name).write_text("keep\n")
    strata.keyboard.press("F5")
    strata.select_entry_with_keyboard(name)
    bounds = strata.window.window_bounds()
    width = 420 if mode == "Columns" else 640
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, width, 300)
    strata.wait(lambda: strata.window.window_bounds().width == width, "a narrow window")
    strata.keyboard.press("F2")
    field = rename_field(strata)
    text = Atspi.Accessible.get_text_iface(field.accessible)
    assert text is not None
    selection = Atspi.Text.get_selection(text, 0)
    assert (selection.start_offset, selection.end_offset) == (0, len(name) - 4)

    def assert_editor_constrained():
        bounds = field.window_bounds()
        window = strata.window.window_bounds()
        pane = strata.pane().window_bounds()
        assert bounds.height > 0 and pane.x <= bounds.x < window.width
        # GTK 4.14 exports a padded origin with the border-box width. The Rust
        # fixture checks exact GtkText/caret bounds; here reject oversized editors.
        assert 0 < bounds.width <= window.width - pane.x

    assert field.window_bounds().width > 0
    strata.keyboard.press("End")
    strata.wait(lambda: Atspi.Text.get_caret_offset(text) == len(name), "End to reach the extension")
    assert Atspi.Text.get_n_selections(text) == 0
    if request.config.getoption("--keep-artifacts"):
        collector = ArtifactCollector(test_name=request.node.name)
        capture(strata.display.display, collector.directory / "after-end.png")
    assert_editor_constrained()
    strata.keyboard.type_text("-final")
    strata.wait(lambda: field.text == name + "-final", "typing after the extension")
    strata.wait(lambda: Atspi.Text.get_caret_offset(text) == len(name) + 6, "the caret to follow typing")
    assert_editor_constrained()
    strata.keyboard.press_repeatedly("BackSpace", 6)
    strata.wait(lambda: field.text == name, "Backspace to remove the appended text")
    strata.keyboard.press("Left")
    strata.wait(lambda: Atspi.Text.get_caret_offset(text) == len(name) - 1, "Left to move the caret")
    strata.keyboard.press("Right")
    strata.wait(lambda: Atspi.Text.get_caret_offset(text) == len(name), "Right to return to the end")
    assert_editor_constrained()
    strata.pointer.click(field)
    strata.wait(lambda: 0 < Atspi.Text.get_caret_offset(text) < len(name), "clicking inside the visible name to move the caret")
    assert_editor_constrained()
    strata.keyboard.press("End")
    strata.wait(lambda: Atspi.Text.get_caret_offset(text) == len(name), "End after clicking")
    strata.keyboard.press("Return")
    wait_for_edit_closed(strata)
    assert strata.fixture.path(name).read_text() == "keep\n"
    assert not strata.fixture.path(name + "-final").exists()


def rename_field(strata):
    return strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable", "focused"}),
        "the rename editor to take focus",
    )


def start_creation(strata, kind, via_menu=True):
    if via_menu:
        strata.pointer.right_click(strata.pane(), at=strata.background_point())
        strata.choose_menu_item("New File" if kind == "file" else "New Folder")
    else:
        strata.keyboard.press("ctrl+shift+n")
    return rename_field(strata)


def begin_edit(strata, kind, new):
    strata.select_entry("readme.md")
    if new:
        field = start_creation(strata, kind, via_menu=kind == "file")
        original = "new " + kind
    else:
        original = "todo.txt" if kind == "file" else "archive"
        strata.select_entry_with_keyboard(original)
        strata.keyboard.press("F2")
        field = rename_field(strata)
        if kind == "file":
            strata.keyboard.press("ctrl+a")
    strata.wait(lambda: field.text == original, "the original name to appear")
    path = strata.fixture.path(original)
    assert path.is_file() if kind == "file" else path.is_dir()
    return field, original


def wait_for_edit_closed(strata):
    strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable"}) is None,
        "the name editor to close",
    )
    assert "Gtk-CRITICAL" not in strata.application.log()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind,via_menu", [("folder", False), ("folder", True), ("file", True)])
def test_new_item_exists_before_typing_and_backspace_clears_its_selected_name(strata, mode, kind, via_menu):
    strata.select_entry("readme.md")
    field = start_creation(strata, kind, via_menu)
    original = "new " + kind
    strata.wait(lambda: field.text == original, "the default name to appear")
    path = strata.fixture.path(original)
    assert path.is_dir() if kind == "folder" else path.is_file()
    if kind == "file":
        assert path.read_bytes() == b""
    strata.keyboard.press("End")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.press("BackSpace")
    strata.wait(lambda: field.text == "", "Ctrl+A and Backspace to clear the entire default name")
    strata.keyboard.press("Return")
    wait_for_edit_closed(strata)
    strata.entry(original)
    assert path.exists()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", KINDS)
def test_new_item_uses_the_first_free_number_without_overwriting(strata, mode, kind):
    base = "new " + kind
    strata.fixture.path(base).write_text("keep\n")
    strata.fixture.path(base + " (1)").symlink_to("missing")
    strata.fixture.path(base + " (2)").mkdir()
    strata.select_entry("readme.md")
    field = start_creation(strata, kind)
    strata.wait(lambda: field.text == base + " (3)", "the first available numbered name")
    created = strata.fixture.path(base + " (3)")
    assert created.is_dir() if kind == "folder" else created.is_file()
    strata.keyboard.press("Escape")
    wait_for_edit_closed(strata)
    assert strata.fixture.path(base).read_text() == "keep\n"
    assert strata.fixture.path(base + " (1)").is_symlink()
    assert strata.fixture.path(base + " (2)").is_dir()
    assert created.exists()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("new", [False, True], ids=["existing", "new"])
@pytest.mark.parametrize("target", ["file", "folder", "background", "sidebar", "tab", "enter"])
def test_leaving_a_valid_name_commits_it(strata, mode, kind, new, target):
    if kind == "folder" and not new:
        strata.fixture.path("archive/marker.txt").write_text("keep\n")
    field, original = begin_edit(strata, kind, new)
    original_path = strata.fixture.path(original)
    contents = original_path.read_bytes() if kind == "file" else None
    strata.keyboard.type_text("renamed.item")
    strata.wait(lambda: field.text == "renamed.item", "typing to replace the selection")
    if target == "sidebar":
        strata.pointer.click(strata.sidebar_button("Home"))
    elif target == "background":
        strata.pointer.click(strata.pane(), at=strata.background_point())
    elif target in ("tab", "enter"):
        strata.keyboard.press("Tab" if target == "tab" else "Return")
    else:
        strata.pointer.click(strata.entry("readme.md" if target == "file" else "documents"))
    renamed = strata.fixture.path("renamed.item")
    strata.wait(renamed.exists, "the rename on disk")
    wait_for_edit_closed(strata)
    assert not original_path.exists()
    if kind == "file":
        assert renamed.read_bytes() == contents
    else:
        assert renamed.is_dir()
        if not new:
            assert (renamed / "marker.txt").read_text() == "keep\n"
    if target == "sidebar":
        strata.wait_for_directory(strata.environment.home.name)
        assert not (strata.environment.home / "renamed.item").exists()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("new", [False, True], ids=["existing", "new"])
@pytest.mark.parametrize("action", ["enter", "click"])
def test_invalid_names_retain_the_original(strata, mode, kind, new, action):
    name = "bad/name"
    field, original = begin_edit(strata, kind, new)
    path = strata.fixture.path(original)
    contents = path.read_bytes() if kind == "file" else None
    strata.keyboard.press("BackSpace")
    strata.keyboard.type_text(name)
    strata.wait(lambda: field.text == name, "the proposed name to appear")
    if action == "enter":
        strata.keyboard.press("Return")
    else:
        strata.pointer.click(strata.pane(), at=strata.background_point())
    wait_for_edit_closed(strata)
    strata.entry(original)
    assert path.exists()
    if kind == "file":
        assert path.read_bytes() == contents
    assert not strata.fixture.path("bad").exists()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("new", [False, True], ids=["existing", "new"])
def test_escape_preserves_the_original_name(strata, mode, kind, new):
    field, original = begin_edit(strata, kind, new)
    strata.keyboard.type_text("discarded")
    strata.wait(lambda: field.text == "discarded", "the proposed name to appear")
    strata.keyboard.press("Escape")
    wait_for_edit_closed(strata)
    strata.entry(original)
    assert strata.fixture.path(original).exists()
    assert not strata.fixture.path("discarded").exists()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", KINDS)
def test_new_item_clears_a_filter_that_would_hide_its_editor(strata, mode, kind):
    strata.select_entry("readme.md")
    strata.keyboard.press("ctrl+f")
    filter_field = strata.editable_field()
    strata.keyboard.type_text("no-matching-entry")
    strata.wait(lambda: filter_field.text == "no-matching-entry", "the filter query")
    strata.wait(lambda: strata.entry_names() == [], "the filter to hide existing entries")
    field = start_creation(strata, kind, via_menu=kind == "file")
    original = "new " + kind
    strata.wait(lambda: field.text == original, "the visible rename editor")
    assert filter_field.text == ""
    assert strata.fixture.path(original).exists()
    strata.keyboard.press("Escape")
    strata.entry(original)


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", KINDS)
def test_repeated_renames_select_the_folder_name_or_file_stem(strata, mode, kind):
    original = "archive" if kind == "folder" else "todo.txt"
    for index in range(1, 4):
        strata.select_entry("readme.md")
        strata.select_entry_with_keyboard(original)
        strata.keyboard.press("F2")
        field = rename_field(strata)
        typed = f"archive.v{index}" if kind == "folder" else f"version-{index}"
        replacement = typed if kind == "folder" else typed + ".txt"
        strata.keyboard.type_text(typed)
        strata.wait(lambda: field.text == replacement, "the intended part of the name to be replaced")
        strata.keyboard.press("Return")
        strata.wait(lambda: strata.fixture.path(replacement).exists(), "the renamed entry")
        strata.entry(replacement)
        assert not strata.fixture.path(original).exists()
        if kind == "file":
            assert strata.fixture.path(replacement).read_text() == "todo\n"
        original = replacement
