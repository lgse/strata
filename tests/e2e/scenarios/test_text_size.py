# SPDX-License-Identifier: MIT
"""Custom typography remains editable and usable across browser presentations."""

import pytest

from harness.artifacts import ArtifactCollector
from harness.modes import ALL_MODES


@pytest.mark.preferences(text_size=17)
def test_settings_text_size_keeps_switches_inside_the_page(strata, request):
    settings = strata.window.find(role="button", name="Settings")
    assert settings is not None and settings.activate()
    for pixels, next_pixels, width in [(17, 11, 1000), (11, 32, 1000), (32, None, 640)]:
        bounds = strata.window.screen_bounds()
        strata.keyboard.connection.resize_surface(bounds.width, bounds.height, width, 600)
        strata.wait(lambda: strata.window.screen_bounds().width == width, "resized settings window")
        general = strata.wait(
            lambda: strata.window.find(role="button", name="General"),
            "General settings navigation",
        )
        assert general.activate()
        toggle = strata.wait(
            lambda: next(
                (node for node in strata.window.find_all(name="Folder peeking")
                 if node.role in {"check box", "toggle button", "switch"}),
                None,
            ),
            "visible folder-peeking switch",
        )
        toggle = _reveal_page_control(strata, "Folder peeking", role=toggle.role)
        scroll = next(node for node in toggle.ancestors() if node.role == "scroll pane")
        bounds, viewport = toggle.screen_bounds(), scroll.screen_bounds()
        assert viewport.x <= bounds.x
        assert bounds.x + bounds.width <= viewport.x + viewport.width
        was_checked = toggle.has_state("checked")
        strata.pointer.click(toggle)
        strata.wait(lambda: toggle.has_state("checked") != was_checked, "resized switch responds to pointer")
        strata.pointer.click(toggle)
        strata.wait(lambda: toggle.has_state("checked") == was_checked, "restore folder peeking")
        if request.config.getoption("--keep-artifacts"):
            strata.screenshot(
                ArtifactCollector(test_name=f"settings-text-size-{pixels}").directory
                / "general.png"
            )
        _reveal_page_control(strata, "Configure…")
        if request.config.getoption("--keep-artifacts"):
            strata.screenshot(
                ArtifactCollector(test_name=f"settings-text-size-{pixels}").directory
                / "configure.png"
            )
        theme = strata.window.find(role="button", name="Appearance settings")
        assert theme is not None and theme.activate()
        control = strata.wait(
            lambda: strata.window.find(role="spin button", name="Text size in pixels", rendered=False),
            "numeric text-size control",
        )
        control = _reveal_page_control(strata, "Text size in pixels", role="spin button")
        strata.wait(lambda: _inside_scroll_view(control), "text-size control scrolled into view")
        # GTK exposes the spin button as one accessible value, not separate buttons.
        for fraction, expected in [(1, pixels - 1), (7, pixels)]:
            bounds = control.screen_bounds()
            strata.pointer.click(
                control,
                at=(bounds.x + bounds.width * fraction // 8, bounds.y + bounds.height // 2),
            )
            strata.wait(
                lambda: strata.environment.read_preferences().get("text_size")
                == str(expected),
                "left decrement and right increment buttons",
            )
        if request.config.getoption("--keep-artifacts"):
            strata.screenshot(
                ArtifactCollector(test_name=f"settings-text-size-{pixels}").directory
                / "selector.png"
            )
        if next_pixels is not None:
            strata.pointer.click(control)
            strata.keyboard.press("ctrl+a")
            strata.keyboard.type_text(str(next_pixels))
            strata.keyboard.press("Return")
            strata.wait(
                lambda: strata.environment.read_preferences().get("text_size")
                == str(next_pixels),
                "updated settings text",
            )
        else:
            strata.keyboard.press("ctrl+0")
            strata.wait(
                lambda: strata.environment.read_preferences().get("text_size") == "13",
                "Reset to restore the default text size",
            )
    updates = strata.window.find(role="button", name="Updates")
    assert updates is not None and updates.activate()
    _reveal_page_control(strata, "Check now")
    if request.config.getoption("--keep-artifacts"):
        strata.screenshot(
            ArtifactCollector(test_name="settings-updates").directory / "check-now.png"
        )


def _reveal_page_control(strata, name, role="button"):
    node = strata.wait(lambda: strata.window.find(role=role, name=name, rendered=False), f"{name} control")
    scroll = next(parent for parent in node.ancestors() if parent.role == "scroll pane")

    def revealed():
        if _inside_scroll_view(node):
            return True
        viewport = scroll.screen_bounds()
        # Stay in the content gutter: the scrollbar scrolls by whole pages,
        # which can jump over a control without ever showing all of it.
        # The nested theme library ends before this gutter.
        at = (viewport.x + viewport.width - 16, viewport.y + viewport.height // 2)
        bounds = node.screen_bounds()
        strata.pointer.scroll(
            at, clicks=1, down=bounds.y + bounds.height > viewport.y + viewport.height
        )
        return False

    strata.wait(revealed, f"reachable {name} control")
    return strata.settle(node)


def _inside_scroll_view(node):
    scroll = next(parent for parent in node.ancestors() if parent.role == "scroll pane")
    bounds, viewport = node.screen_bounds(), scroll.screen_bounds()
    return (
        bounds.width > 0
        and bounds.height > 0
        and viewport.x <= bounds.x
        and viewport.y <= bounds.y
        and bounds.x + bounds.width <= viewport.x + viewport.width
        and bounds.y + bounds.height <= viewport.y + viewport.height
    )


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
        lambda: strata.window.find(role="button", name="Appearance settings"),
        "appearance settings",
    )
    assert theme.activate()
    control = strata.wait(
        lambda: strata.window.find(role="spin button", name="Text size in pixels", rendered=False),
        "numeric text size control",
    )
    control = _reveal_page_control(strata, "Text size in pixels", role="spin button")
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
