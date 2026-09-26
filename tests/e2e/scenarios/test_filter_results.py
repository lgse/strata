# SPDX-License-Identifier: GPL-3.0-or-later
import tomllib
from pathlib import Path

import pytest
from PIL import Image, ImageColor

from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES


@pytest.fixture
def fixture_tree():
    fixture = FixtureTree.create({
        "match-note.txt": "root decoy\n",
        "alpha": {"match-note.txt": "alpha source\n"},
        "beta": {"match-note.txt": "beta source\n", "only-match.txt": "beta source\n"},
        "match-note-other.md": "other match\n",
        "destination": {},
        "thumb.txt": "thumbnail search companion",
    })
    Image.new("RGB", (64, 64), (230, 40, 60)).save(fixture.path("beta/thumb.png"))
    try:
        yield fixture
    finally:
        fixture.cleanup()


def result_rows(strata):
    container = strata.window.find(role="list", name="Search results")
    role = "list item"
    if container is None:
        container = strata.window.find(role="table", name="Search results")
        role = "table cell"
    if container is None:
        return strata.window.find_all(role="list item")
    return container.find_all(role=role)


def result(strata, path):
    for row in result_rows(strata):
        if row.name == Path(path).name and any(
            label.name.endswith(path) for label in row.find_all(role="label")
        ):
            return row
    return None


def filter_results(strata, query="match-note", count=4, directory=None):
    strata.select_entry("match-note.txt", directory)
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text(query)
    strata.wait(lambda: len(strata.matches()) == count, "all recursive matches")
    return field


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("route", ["shift", "keyboard-range", "select-all", "marquee"])
@pytest.mark.parametrize("recursive", [
    pytest.param(False, marks=pytest.mark.preferences(filter_include_subfolders=False)),
    pytest.param(True, marks=pytest.mark.preferences(filter_include_subfolders=True)),
])
def test_filtered_results_support_group_selection(strata, mode, route, recursive):
    count = 4 if recursive else 2
    filter_results(strata, count=count)
    rows = [row for row in result_rows(strata) if "match-note" in row.name]
    assert len(rows) == count
    if route == "marquee":
        first = rows[0].screen_bounds()
        last = rows[-1].screen_bounds()
        strata.pointer.drag_points(
            (last.center[0], last.y + last.height + 25),
            (first.x + 15, first.y + 2),
        )
    else:
        strata.pointer.click(rows[0], modifiers=("ctrl",))
        if route == "shift":
            strata.pointer.click(rows[-1], modifiers=("shift",))
        else:
            strata.keyboard.press("ctrl+f")
            strata.keyboard.press("Down")
            if route == "keyboard-range":
                step = "shift+Right" if mode == "Icons" else "shift+Down"
                for _ in range(count - 1):
                    strata.keyboard.press(step)
            else:
                strata.keyboard.press("ctrl+a")
    strata.wait(
        lambda: all(row.has_state("selected") for row in rows),
        "all filtered results to be selected",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("route", ["keyboard", "context-menu"])
@pytest.mark.parametrize("recursive", [
    pytest.param(False, marks=pytest.mark.preferences(filter_include_subfolders=False)),
    pytest.param(True, marks=pytest.mark.preferences(filter_include_subfolders=True)),
])
@pytest.mark.preferences(list_file_clicks=2, grid_file_clicks=2, explorer_file_clicks=2)
def test_filtered_properties_uses_visible_selection(strata, mode, route, recursive):
    strata.select_entry("match-note.txt")
    strata.pointer.click(strata.entry("match-note-other.md"), modifiers=("ctrl",))
    strata.keyboard.press("ctrl+f")
    strata.keyboard.type_text("match-note")
    strata.wait(lambda: len(strata.matches()) == (4 if recursive else 2), "matching files")
    rows = result_rows(strata)
    first = result(strata, "alpha/match-note.txt") if recursive else next(row for row in rows if row.name == "match-note.txt")
    second = next(row for row in rows if row.name == "match-note-other.md")
    assert first is not None and second is not None
    for row in rows:
        if row.has_state("selected"):
            strata.pointer.click(row, modifiers=("ctrl",))
    strata.pointer.click(first, modifiers=("ctrl",))
    strata.pointer.click(second, modifiers=("ctrl",))
    strata.wait(lambda: first.has_state("selected") and second.has_state("selected"), "selected hits")
    if route == "keyboard":
        strata.keyboard.press("alt+Return")
    else:
        strata.pointer.right_click(first)
        strata.wait(strata.context_menu, "selected results menu")
        strata.choose_menu_item("Properties")
    dialog = strata.wait_for_dialog()
    strata.wait(lambda: dialog.find(role="label", name="≥ 2 files, ≥ 0 folders"), "selected search hits")
    assert dialog.find(role="label", name="≥ 0 B")
    strata.keyboard.press("Escape")
    strata.wait(lambda: not dialog.is_rendered(), "Properties to close")
    strata.pointer.click(second, modifiers=("ctrl",))
    strata.wait(lambda: first.has_state("selected") and not second.has_state("selected"), "one selected hit")
    strata.keyboard.press("alt+Return")
    dialog = strata.wait_for_dialog()
    strata.wait(lambda: dialog.find(role="label", name="13 B" if recursive else "11 B"), "single result size")
    assert dialog.find(role="label", name="OPENS WITH")


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("operation", ["drag", "copy"])
def test_filtered_control_selection_operates_on_the_selected_group(strata, mode, operation):
    filter_results(strata, query="match", count=5)
    first = result(strata, "beta/only-match.txt")
    second = result(strata, "match-note-other.md")
    assert first is not None and second is not None
    strata.pointer.click(first, modifiers=("ctrl",))
    strata.pointer.click(second, modifiers=("ctrl",))
    strata.pointer.click(first, modifiers=("ctrl",))
    strata.wait(lambda: not first.has_state("selected"), "Ctrl-click to deselect a result")
    strata.pointer.click(first, modifiers=("ctrl",))
    strata.wait(
        lambda: first.has_state("selected") and second.has_state("selected"),
        "both filtered files to be selected",
    )
    if operation == "drag":
        strata.pointer.drag(first, strata.sidebar_button("Home"))
    else:
        strata.pointer.right_click(first)
        strata.wait(strata.context_menu, "the selected group menu")
        assert first.has_state("selected") and second.has_state("selected")
        strata.choose_menu_item("Copy")
        strata.keyboard.press("ctrl+l")
        strata.keyboard.press("ctrl+a")
        strata.keyboard.type_text(str(strata.environment.home))
        strata.keyboard.press("Return")
        strata.wait_for_directory(strata.environment.home.name)
        strata.keyboard.press("ctrl+v")
    for path, contents in [("beta/only-match.txt", "beta source\n"),
                           ("match-note-other.md", "other match\n")]:
        destination = strata.environment.home / Path(path).name
        strata.wait(lambda: destination.exists(), "the selected file to arrive in Home")
        assert destination.read_text() == contents
        assert strata.fixture.path(path).exists() == (operation == "copy")
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("shortcut", ["ctrl+c", "ctrl+x"])
@pytest.mark.parametrize("hints", [
    pytest.param(False, marks=pytest.mark.preferences(show_keybinding_hints=False)),
    pytest.param(True, marks=pytest.mark.preferences(show_keybinding_hints=True)),
])
def test_filtered_keyboard_clipboard_keeps_status_visible(strata, mode, shortcut, hints):
    filter_results(strata, query="match", count=5)
    for path in ["beta/only-match.txt", "match-note-other.md"]:
        row = result(strata, path)
        assert row is not None
        strata.pointer.click(row, modifiers=("ctrl",))
    strata.keyboard.press(shortcut)
    strata.wait(
        lambda: strata.window.find(role="label", name="Files on clipboard"),
        "the file clipboard status badge",
    )
    assert bool(strata.window.find(role="button", name="F1  Shortcuts")) == hints
    assert strata.fixture.path("beta/only-match.txt").exists()
    assert strata.fixture.path("match-note-other.md").exists()
    strata.keyboard.press("ctrl+l")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(strata.fixture.path("destination")))
    strata.keyboard.press("Return")
    strata.wait_for_directory("destination")
    strata.keyboard.press("ctrl+v")
    for path, contents in [("beta/only-match.txt", "beta source\n"),
                           ("match-note-other.md", "other match\n")]:
        destination = strata.fixture.path(f"destination/{Path(path).name}")
        strata.wait(lambda: destination.exists(), "the clipboard file to arrive")
        assert destination.read_text() == contents
        assert strata.fixture.path(path).exists() == (shortcut == "ctrl+c")
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"


@pytest.mark.parametrize("mode", ALL_MODES)
def test_filter_right_arrow_moves_the_text_cursor(strata, mode):
    strata.switch_view(mode)
    field = filter_results(strata)
    strata.keyboard.press("Home")
    strata.keyboard.press("Right")
    strata.keyboard.type_text("X")
    strata.wait(lambda: field.text == "mXatch-note", "Right to move the filter caret")


def test_filter_text_selection_uses_the_active_theme(strata, tmp_path):
    field = filter_results(strata)
    strata.keyboard.press("ctrl+a")
    settings = tomllib.loads(strata.environment.settings_path.read_text())
    catalog = Path(__file__).resolve().parents[3] / "data/themes/catalog.toml"
    themes = tomllib.loads(catalog.read_text())["themes"]
    theme = next(theme for theme in themes if theme["id"] == settings["theme"])
    accent = ImageColor.getrgb(theme["accent"])

    def selected_text_has_theme_background():
        bounds = field.screen_bounds()
        capture = strata.screenshot(tmp_path / "filter-selection.png")
        with Image.open(capture) as image:
            pixels = image.convert("RGB").crop((
                bounds.x + 2, bounds.y + 2,
                bounds.x + bounds.width - 2, bounds.y + bounds.height - 2,
            ))
            return sum(count for count, color in pixels.getcolors(pixels.width * pixels.height)
                       if color == accent) > 100

    strata.wait(selected_text_has_theme_background, "theme-colored filter text selection")


@pytest.mark.parametrize("mode", [mode for mode in ALL_MODES if mode.id != "list"])
@pytest.mark.parametrize("trigger,query,count,target", [
    ("pointer", "match-note", 4, "beta/match-note.txt"),
    ("keyboard", "match-note", 4, "beta/match-note.txt"),
    ("keyboard", "only-match", 1, "beta/only-match.txt"),
])
def test_filtered_item_menu_actions_use_the_real_location(strata, mode, trigger, query, count, target):
    directory = "beta" if mode == "Columns" and count == 1 else None
    if directory:
        strata.open_directory(directory)
    field = filter_results(strata, query, count, directory)
    row = strata.wait(lambda: result(strata, target), "the beta result")
    if mode == "Columns" and count == 1:
        strata.hover_pane(strata.fixture.root.name)
    if trigger == "pointer":
        strata.pointer.right_click(row)
        strata.wait(strata.context_menu, "the result menu")
        strata.keyboard.press("Escape")
        strata.wait(lambda: strata.context_menu() is None, "the pointer result menu to close")
        strata.wait(lambda: row.has_state("focused"), "focus to return to the right-clicked result")
        strata.pointer.right_click(row)
    else:
        strata.keyboard.press("Down")
        strata.wait(lambda: not field.has_state("focused"), "Down to leave the filter input")
        result_step = "Right" if mode == "Icons" else "Down"
        for _ in range(count):
            if row.has_state("focused"):
                break
            strata.keyboard.press(result_step)
        strata.wait(lambda: row.has_state("focused"), "keyboard focus on the actual result")
        strata.keyboard.press("ctrl+f")
        strata.wait(lambda: field.has_state("focused"), "Ctrl+F to refocus the query")
        assert field.text == query
        strata.keyboard.press("Down")
        strata.wait(lambda: row.has_state("focused"), "Down to resume the selected result")
        for _ in range(count):
            strata.keyboard.press("Up")
            if field.has_state("focused"):
                break
        strata.wait(lambda: field.has_state("focused"), "Up from the first result to the query")
        assert field.text == query
        strata.keyboard.press("Down")
        strata.wait(lambda: not field.has_state("focused"), "Down to reenter results")
        for _ in range(count):
            if row.has_state("focused"):
                break
            strata.keyboard.press(result_step)
        strata.wait(lambda: row.has_state("focused"), "keyboard result focus after the round trip")
        strata.keyboard.press("Menu")
        strata.wait(strata.context_menu, "the keyboard result menu")
        strata.keyboard.press("Escape")
        strata.wait(lambda: strata.context_menu() is None, "the result menu to close")
        strata.wait(lambda: row.has_state("focused"), "focus to return to the result")
        strata.keyboard.press("shift+F10")
    strata.wait(strata.context_menu, "the result menu")
    assert row.has_state("selected")
    assert "Quick preview" in strata.menu_items()
    assert "Open file location" in strata.menu_items()
    assert "New Folder" not in strata.menu_items()
    if trigger == "keyboard":
        strata.keyboard.press("Home")
        strata.keyboard.press("Up")
        assert not field.has_state("focused")
        strata.keyboard.press("Home")
        for _ in strata.menu_items():
            if strata.menu_item("Properties").has_state("focused"):
                break
            strata.keyboard.press("Down")
        assert strata.menu_item("Properties").has_state("focused")
        strata.keyboard.press("Return")
    else:
        strata.choose_menu_item("Properties")
    dialog = strata.wait_for_dialog()
    assert target in dialog.dump()
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "result Properties to close")
    strata.wait(lambda: row.has_state("focused"), "Properties to restore the actual search result")
    assert row.has_state("selected")
    assert field.text == query
    if trigger == "pointer":
        strata.pointer.right_click(row)
    else:
        strata.keyboard.press("Menu")
    strata.wait(strata.context_menu, "the restored result menu")
    strata.choose_menu_item("Quick preview")
    strata.wait(lambda: strata.preview_shows("beta source"), "preview of the nested result")
    assert row.has_state("selected")
    assert field.text == query
    strata.keyboard.press("ctrl+f")
    strata.wait(lambda: field.has_state("focused"), "Ctrl+F to return from the preview")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is None, "Space to close preview")
    assert field.text == query
    strata.pointer.right_click(result(strata, target))
    strata.wait(strata.context_menu, "the selected result menu")
    strata.choose_menu_item("Copy")
    assert field.text == query
    strata.keyboard.press("ctrl+l")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(strata.fixture.path("destination")))
    strata.keyboard.press("Return")
    strata.wait_for_directory("destination")
    strata.keyboard.press("ctrl+v")
    destination = strata.fixture.path(f"destination/{Path(target).name}")
    strata.wait(lambda: destination.exists() and destination.read_text() == "beta source\n",
                "the actual nested file to be copied")
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"
    assert strata.fixture.path("alpha/match-note.txt").read_text() == "alpha source\n"


@pytest.mark.parametrize("mode", ALL_MODES)
def test_query_updates_retain_selection_focus_preview_and_background_menu(strata, mode):
    field = filter_results(strata)
    row = strata.wait(lambda: result(strata, "beta/match-note.txt"), "the beta result")
    strata.pointer.click(row, modifiers=("ctrl",))
    strata.pointer.click(field)
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("beta source"), "the selected preview")
    for query, count in [("match-note.t", 3), ("match-note", 4)] * 2:
        strata.keyboard.press("ctrl+a")
        strata.keyboard.type_text(query)
        strata.wait(lambda: len(strata.matches()) == count, "the updated result count")
        assert result(strata, "beta/match-note.txt").has_state("selected")
        assert field.has_state("focused")
        assert field.text == query
        assert strata.preview_shows("beta source")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is None, "Space to close after updates")
    assert field.text == "match-note"
    strata.pointer.right_click(strata.pane(), at=strata.background_point())
    strata.wait(strata.context_menu, "the empty-space menu")
    assert "New Folder" in strata.menu_items()
    assert "Quick preview" not in strata.menu_items()


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("trigger,focus_filter", [
    ("menu", False),
    ("F2", False),
    ("F2", True),
    ("ctrl+r", False),
    ("ctrl+r", True),
])
def test_filtered_rename_targets_the_nested_duplicate(strata, mode, trigger, focus_filter):
    field = filter_results(strata)
    row = strata.wait(lambda: result(strata, "beta/match-note.txt"), "the beta result")
    if trigger == "menu":
        strata.pointer.right_click(row)
        strata.wait(strata.context_menu, "the result menu")
        strata.choose_menu_item("Rename")
    else:
        strata.pointer.click(row, modifiers=("ctrl",))
        if focus_filter:
            strata.pointer.click(field)
        strata.keyboard.press(trigger)
    strata.editable_field()
    assert strata.dialog() is None
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable"}) is None,
        "inline rename to close",
    )
    strata.wait(
        lambda: result(strata, "beta/match-note.txt").has_state("focused"),
        "focus to return to the originating result",
    )
    assert result(strata, "beta/match-note.txt").has_state("selected")
    assert field.text == "match-note"
    assert strata.fixture.path("beta/match-note.txt").read_text() == "beta source\n"
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("renamed.txt")
    strata.keyboard.press("Return")
    renamed = strata.fixture.path("beta/renamed.txt")
    strata.wait(lambda: renamed.exists() and renamed.read_text() == "beta source\n", "the nested rename")
    assert not strata.fixture.path("beta/match-note.txt").exists()
    assert strata.fixture.path("alpha/match-note.txt").read_text() == "alpha source\n"
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"
    assert field.text == "match-note"
    strata.wait(lambda: result(strata, "beta/match-note.txt") is None, "the stale hit to disappear")
    strata.pointer.click(field)
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("match-note.t")
    strata.wait(lambda: len(strata.matches()) == 2, "only the surviving matches after a query change")
    assert result(strata, "beta/match-note.txt") is None


@pytest.mark.parametrize("mode", ALL_MODES)
def test_matching_rename_stays_searchable_at_the_real_parent(strata, mode):
    field = filter_results(strata)
    row = strata.wait(lambda: result(strata, "beta/match-note.txt"), "the beta result")
    strata.pointer.click(row, modifiers=("ctrl",))
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("match-note-renamed.txt")
    strata.keyboard.press("Return")
    renamed = strata.fixture.path("beta/match-note-renamed.txt")
    strata.wait(lambda: renamed.exists(), "the nested rename")
    assert renamed.read_text() == "beta source\n"
    assert not strata.fixture.path("beta/match-note.txt").exists()
    assert strata.fixture.path("alpha/match-note.txt").read_text() == "alpha source\n"
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"
    assert field.text == "match-note"
    strata.wait(lambda: result(strata, "beta/match-note-renamed.txt"), "the matching renamed result")
    strata.wait(lambda: result(strata, "beta/match-note.txt") is None, "the old result to leave")
    strata.pointer.click(field)
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("nothing-matches")
    strata.wait(lambda: not strata.matches(), "the empty query result")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("renamed")
    row = strata.wait(lambda: result(strata, "beta/match-note-renamed.txt"), "the fresh index result")
    strata.pointer.right_click(row)
    items = strata.menu_items()
    assert items.index("Quick preview") < items.index("Open file location")
    strata.choose_menu_item("Open file location")
    strata.wait_for_directory("beta")
    strata.wait_for_selection(["match-note-renamed.txt"], "beta")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_filtered_properties_rename_opens_the_result_inline(strata, mode):
    field = filter_results(strata)
    row = strata.wait(lambda: result(strata, "beta/match-note.txt"), "the beta result")
    strata.pointer.right_click(row)
    strata.choose_menu_item("Properties")
    dialog = strata.wait_for_dialog()
    strata.pointer.click(dialog.find(role="button", name="Rename"))
    strata.wait(lambda: strata.dialog() is None, "properties dialog to close")
    rename = strata.editable_field()
    assert rename.text == "match-note.txt"
    assert field.text == "match-note"
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable"}) is None,
        "inline rename to close",
    )
    assert strata.fixture.path("beta/match-note.txt").read_text() == "beta source\n"


@pytest.mark.parametrize("mode", ALL_MODES)
def test_delete_trashes_filtered_result_without_touching_hidden_selection(strata, mode):
    filter_results(strata)
    row = strata.wait(lambda: result(strata, "beta/match-note.txt"), "the beta result")
    strata.pointer.click(row, modifiers=("ctrl",))
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("Escape")
    strata.wait(lambda: result(strata, "beta/match-note.txt").has_state("focused"), "result focus")
    strata.keyboard.press("Delete")
    strata.wait(
        lambda: not strata.fixture.path("beta/match-note.txt").exists(),
        "the selected result to be trashed",
    )
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"
    assert strata.fixture.path("alpha/match-note.txt").read_text() == "alpha source\n"


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("query", ["", "no-such-result"])
def test_filter_rename_shortcuts_do_not_target_the_hidden_directory_selection(strata, mode, query):
    strata.select_entry("match-note.txt")
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    if query:
        strata.keyboard.type_text(query)
        strata.wait(lambda: len(strata.matches()) == 0, "no matching results")
        strata.keyboard.press("Down")
        assert field.has_state("focused")
        assert field.text == query
    for shortcut in ["F2", "ctrl+r"]:
        strata.keyboard.press(shortcut)
        strata.settle(field)
        assert strata.dialog() is None
        assert field.has_state("focused")
        assert field.text == query
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"


@pytest.mark.parametrize("mode", ALL_MODES)
def test_filtered_thumbnail_stays_rendered_across_updates_and_rename(strata, mode, tmp_path):
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text("thumb")
    strata.wait(lambda: len(strata.matches()) == 2, "image and text results")
    row = result(strata, "beta/thumb.png")
    assert row is not None
    strata.pointer.click(row, modifiers=("ctrl",))
    strata.pointer.click(field)
    icon = row.find(role="image")
    assert icon is not None

    def thumbnail_pixel():
        bounds = strata.settle(icon).screen_bounds()
        capture = strata.screenshot(tmp_path / "thumbnail.png")
        with Image.open(capture) as image:
            return image.convert("RGB").getpixel(bounds.center)

    strata.wait(lambda: thumbnail_pixel() == (230, 40, 60), "the generated red thumbnail")
    for query, count in [("thumb.p", 1), ("thumb", 2)]:
        strata.keyboard.press("ctrl+a")
        strata.keyboard.type_text(query)
        strata.wait(lambda: len(strata.matches()) == count, "updated image results")
        assert row.has_state("selected")
        # AT-SPI result updates can precede the corresponding rendered frame.
        strata.wait(lambda: thumbnail_pixel() == (230, 40, 60), "the updated red thumbnail")
        assert field.has_state("focused")

    original = strata.fixture.path("beta/thumb.png").read_bytes()
    strata.keyboard.press("Down")
    strata.keyboard.press("F2")
    strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("thumb-renamed.png")
    strata.keyboard.press("Return")
    renamed = strata.fixture.path("beta/thumb-renamed.png")
    strata.wait(lambda: renamed.exists(), "the image rename")
    assert renamed.read_bytes() == original
    assert not strata.fixture.path("beta/thumb.png").exists()
    assert field.text == "thumb"
    row = strata.wait(lambda: result(strata, "beta/thumb-renamed.png"), "the renamed image result")
    icon = row.find(role="image")
    assert icon is not None
    strata.wait(lambda: thumbnail_pixel() == (230, 40, 60), "the renamed red thumbnail")
