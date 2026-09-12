# SPDX-License-Identifier: MIT
"""Settings-wide search keeps input focus while navigating and filtering pages."""

import pytest


@pytest.mark.parametrize("width", [1360, 640], ids=["sidebar", "compact-popover"])
def test_settings_search_filters_navigates_and_clears(strata, width):
    bounds = strata.window.screen_bounds()
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, width, 700)
    strata.wait(lambda: strata.window.screen_bounds().width == width, "resized window")
    settings = strata.window.find(role="button", name="Settings")
    assert settings is not None and settings.activate()
    if width == 640:
        button = strata.wait(
            lambda: strata.window.find(role="button", name="Search settings"),
            "compact settings search",
        )
        strata.pointer.click(button)
    search = strata.wait(
        lambda: strata.window.find(role="text", name="Search settings"),
        "settings search field",
    )
    original = strata.environment.read_preferences()
    strata.pointer.click(search)
    strata.keyboard.type_text("tezt size")
    strata.wait(lambda: search.text == "tezt size", "search retains focus across page changes")
    strata.wait(
        lambda: strata.window.find(role="spin button", name="Text size in pixels"),
        "nearest text-size setting",
    )
    assert strata.window.find(name="Reduce motion") is None
    assert strata.window.find(name="Search themes") is None
    preferences = strata.environment.read_preferences()
    for key in ("folder_peeking", "text_size", "follow_omarchy", "theme"):
        assert preferences.get(key) == original.get(key)

    bounds = strata.window.screen_bounds()
    width = 640 if width == 1360 else 1360
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, width, 700)
    strata.wait(lambda: strata.window.screen_bounds().width == width, "resize with an active query")
    search = strata.wait(
        lambda: strata.window.find(role="text", name="Search settings"),
        "search follows the responsive navigation",
    )
    strata.wait(lambda: search.has_state("focused"), "search input focus survives resize")
    assert search.text == "tezt size"
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("folder peeking")
    strata.wait(lambda: search.text == "folder peeking", "replace global query")
    strata.wait(lambda: strata.window.find(name="Folder peeking"), "matching General setting")
    assert strata.window.find(name="Single-click file previews") is None

    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("unfindablequantumsetting")
    strata.wait(
        lambda: strata.window.find(role="label", name="No settings match your search."),
        "explicit empty results",
    )
    strata.keyboard.press("Escape")
    strata.wait(lambda: search.text == "", "Escape clears the query before closing Settings")
    if width == 640:
        strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(name="Single-click file previews"),
        "unfiltered settings restored",
    )
    assert strata.window.find(role="button", name="Close settings") is not None
