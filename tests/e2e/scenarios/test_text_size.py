# SPDX-License-Identifier: MIT
"""Custom typography remains editable and usable across browser presentations."""

import pytest

from harness.artifacts import ArtifactCollector
from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.preferences(text_size=24)
def test_custom_text_size_shortcuts_numeric_control_and_restart(strata, mode, request):
    strata.switch_view(mode)
    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+=")
    strata.wait(
        lambda: strata.environment.read_preferences().get("text_size") == "25",
        "zoom in to persist a numeric size",
    )
    strata.keyboard.press("ctrl+-")
    strata.wait(
        lambda: strata.environment.read_preferences().get("text_size") == "24",
        "zoom out to restore the custom size",
    )
    strata.keyboard.press("ctrl+0")
    strata.wait(
        lambda: strata.environment.read_preferences().get("text_size") == "13",
        "reset to the default size",
    )
    if request.config.getoption("--keep-artifacts"):
        strata.screenshot(ArtifactCollector(test_name=f"text-size-{mode}").directory / "before.png")
    strata.keyboard.press("F2")
    rename = strata.wait(
        lambda: strata.window.find(role="text", name="Rename"), "inline rename editor"
    )
    strata.keyboard.press("ctrl+=")
    strata.wait(
        lambda: strata.environment.read_preferences().get("text_size") == "14",
        "zoom while renaming without submitting the editor",
    )
    assert rename.alive
    strata.keyboard.press("Escape")
    assert strata.fixture.path("todo.txt").exists()

    settings = strata.window.find(role="button", name="Settings")
    assert settings is not None and settings.activate()
    theme = strata.wait(
        lambda: strata.window.find(role="button", name="Theme & appearance"),
        "appearance settings",
    )
    assert theme.activate()
    control = strata.wait(
        lambda: strata.window.find(role="spin button", name="Text size in pixels"),
        "numeric text size control",
    )
    strata.pointer.click(control)
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("27")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: strata.environment.read_preferences().get("text_size") == "27",
        "an arbitrary typed text size to persist",
    )
    strata.application.stop()
    strata.application.start()
    assert strata.environment.read_preferences()["text_size"] == "27"
    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+=")
    strata.wait(
        lambda: strata.environment.read_preferences().get("text_size") == "28",
        "restart to load the exact custom size",
    )
    if request.config.getoption("--keep-artifacts"):
        strata.screenshot(ArtifactCollector(test_name=f"text-size-{mode}").directory / "after.png")
