# SPDX-License-Identifier: GPL-3.0-or-later
import tomllib
from pathlib import Path

import pytest
from PIL import Image, ImageColor

from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES, SINGLE_PANE_MODES


@pytest.fixture
def fixture_tree():
    fixture = FixtureTree.create({
        "match-note.txt": "root decoy\n",
        "alpha": {"match-note.txt": "alpha source\n"},
        "beta": {"match-note.txt": "beta source\n"},
        "match-note-other.md": "other match\n",
        "destination": {},
        "thumb.txt": "thumbnail search companion",
    })
    Image.new("RGB", (64, 64), (230, 40, 60)).save(fixture.path("beta/thumb.png"))
    try:
        yield fixture
    finally:
        fixture.cleanup()


def result(strata, path):
    for row in strata.window.find_all(role="list item", name="match-note.txt"):
        if any(label.name.endswith(path) for label in row.find_all(role="label")):
            return row
    return None


def filter_results(strata):
    strata.select_entry("match-note.txt")
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text("match-note")
    strata.wait(lambda: len(strata.matches()) == 4, "all recursive matches")
    return field


@pytest.mark.parametrize("mode", ALL_MODES)
def test_filter_text_selection_uses_the_active_theme(strata, mode, tmp_path):
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


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("trigger", ["pointer", "keyboard"])
def test_filtered_item_menu_previews_and_copies_the_real_location(strata, mode, trigger):
    field = filter_results(strata)
    row = strata.wait(lambda: result(strata, "beta/match-note.txt"), "the beta result")
    if trigger == "pointer":
        strata.pointer.right_click(row)
    else:
        strata.pointer.right_click(row)
        strata.wait(strata.context_menu, "the result menu")
        strata.choose_menu_item("Rename")
        strata.wait_for_dialog()
        strata.keyboard.press("Escape")
        strata.wait(lambda: strata.dialog() is None, "rename cancellation")
        strata.wait(lambda: row.has_state("focused"), "the result to own keyboard focus")
        strata.keyboard.press("Menu")
        strata.wait(strata.context_menu, "the keyboard result menu")
        strata.keyboard.press("Escape")
        strata.wait(lambda: strata.context_menu() is None, "the result menu to close")
        strata.wait(lambda: row.has_state("focused"), "focus to return to the result")
        strata.keyboard.press("shift+F10")
    strata.wait(strata.context_menu, "the result menu")
    assert row.has_state("selected")
    assert "Quick preview" in strata.menu_items()
    assert "New Folder" not in strata.menu_items()
    strata.choose_menu_item("Quick preview")
    strata.wait(lambda: strata.preview_shows("beta source"), "preview of the nested result")
    assert field.text == "match-note"
    strata.pointer.click(field)
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is None, "Space to close preview")
    assert field.text == "match-note"
    strata.pointer.right_click(result(strata, "beta/match-note.txt"))
    strata.wait(strata.context_menu, "the selected result menu")
    strata.choose_menu_item("Copy")
    assert field.text == "match-note"
    strata.keyboard.press("ctrl+l")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(strata.fixture.path("destination")))
    strata.keyboard.press("Return")
    strata.wait_for_directory("destination")
    strata.keyboard.press("ctrl+v")
    destination = strata.fixture.path("destination/match-note.txt")
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
    strata.wait_for_dialog()
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "rename dialog to close")
    strata.wait(
        lambda: result(strata, "beta/match-note.txt").has_state("focused"),
        "focus to return to the originating result",
    )
    assert result(strata, "beta/match-note.txt").has_state("selected")
    assert field.text == "match-note"
    assert strata.fixture.path("beta/match-note.txt").read_text() == "beta source\n"
    strata.keyboard.press("Delete")
    strata.settle(result(strata, "beta/match-note.txt"))
    assert strata.dialog() is None
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"
    strata.keyboard.press("F2")
    strata.wait_for_dialog()
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
@pytest.mark.parametrize("query", ["", "no-such-result"])
def test_filter_rename_shortcuts_do_not_target_the_hidden_directory_selection(strata, mode, query):
    strata.select_entry("match-note.txt")
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    if query:
        strata.keyboard.type_text(query)
        strata.wait(lambda: len(strata.matches()) == 0, "no matching results")
    for shortcut in ["F2", "ctrl+r"]:
        strata.keyboard.press(shortcut)
        strata.settle(field)
        assert strata.dialog() is None
        assert field.has_state("focused")
        assert field.text == query
    assert strata.fixture.path("match-note.txt").read_text() == "root decoy\n"


@pytest.mark.parametrize("mode", SINGLE_PANE_MODES)
def test_filtered_thumbnail_stays_rendered_across_updates(strata, mode, tmp_path):
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text("thumb")
    strata.wait(lambda: len(strata.matches()) == 2, "image and text results")
    row = strata.window.find(role="list item", name="thumb.png")
    assert row is not None
    strata.pointer.click(row, modifiers=("ctrl",))
    strata.pointer.click(field)
    icon = row.find(role="image")
    assert icon is not None

    def thumbnail_pixel():
        bounds = icon.screen_bounds()
        capture = strata.screenshot(tmp_path / "thumbnail.png")
        with Image.open(capture) as image:
            return image.convert("RGB").getpixel((bounds.center[0], row.screen_bounds().center[1]))

    strata.wait(lambda: thumbnail_pixel() == (230, 40, 60), "the generated red thumbnail")
    for query, count in [("thumb.p", 1), ("thumb", 2)]:
        strata.keyboard.press("ctrl+a")
        strata.keyboard.type_text(query)
        strata.wait(lambda: len(strata.matches()) == count, "updated image results")
        assert row.has_state("selected")
        assert thumbnail_pixel() == (230, 40, 60)
        assert field.has_state("focused")
