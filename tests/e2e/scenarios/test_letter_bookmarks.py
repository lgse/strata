# SPDX-License-Identifier: MIT
"""10xer letter marks follow the focused directory, not its selected child."""

import json

import pytest
from harness.keyboard_layout import RUSSIAN, US
from harness.modes import ALL_MODES


def marks(strata):
    path = strata.environment.data_home / "strata/letter-bookmarks.json"
    return json.loads(path.read_text())["entries"] if path.exists() else {}


def chord(strata, prefix, letter):
    strata.keyboard.press(prefix)
    strata.keyboard.press(letter)


@pytest.mark.preferences(tenxer_mode=True, type_to_search=False, list_folder_clicks=2)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_letter_bookmarks_save_current_folder_replace_and_survive_restart(strata, mode):
    assert strata.view_mode() == mode
    root = strata.fixture.root
    strata.select_entry("documents", directory=root.name)
    chord(strata, "m", "a")
    strata.wait(lambda: marks(strata).get("a") == root.as_uri(), "current folder mark")
    strata.keyboard.press("Return")
    strata.wait_for_directory("documents")
    strata.select_entry("notes.txt")
    chord(strata, "m", "b")
    documents = strata.fixture.path("documents")
    strata.wait(lambda: marks(strata).get("b") == documents.as_uri(), "second mark")
    chord(strata, "'", "a")
    strata.wait_for_directory(root.name)
    strata.select_entry("readme.md")
    chord(strata, "'", "b")
    strata.wait_for_directory("documents")
    strata.select_entry("notes.txt")
    chord(strata, "m", "a")
    strata.wait(lambda: marks(strata).get("a") == documents.as_uri(), "replaced mark")
    strata.application.stop()
    strata.application.start()
    strata.select_entry("readme.md")
    chord(strata, "'", "a")
    strata.wait_for_directory("documents")


@pytest.mark.preferences(browser_mode="columns", tenxer_mode=True, type_to_search=False)
@pytest.mark.parametrize("layout", [US, RUSSIAN], ids=["us", "russian"])
def test_letter_bookmarks_layout_fallback_and_cancel(strata, keyboard_layout, layout):
    root = strata.fixture.root
    strata.select_entry("readme.md")
    with keyboard_layout(layout):
        chord(strata, "m", "Escape")
        chord(strata, "m", "1")
        chord(strata, "m", "z")
    strata.wait(lambda: marks(strata).get("z") == root.as_uri(), "normalized letter mark")
    assert set(marks(strata)) == {"z"}
    strata.select_entry("documents", directory=root.name)
    strata.keyboard.press("Return")
    strata.wait_for_directory("documents")
    strata.select_entry("notes.txt")
    with keyboard_layout(layout):
        chord(strata, "'", "z")
    strata.wait_for_directory(root.name)
