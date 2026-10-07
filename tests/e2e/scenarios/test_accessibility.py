# SPDX-License-Identifier: MIT
"""Accessibility semantics the rest of the suite — and screen readers — rely on."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES

ROOT_ENTRIES = ["archive", "documents", "pictures", "readme.md", "todo.txt"]
FOLDERS = {"archive", "documents", "pictures"}


@pytest.mark.parametrize("mode", ALL_MODES)
def test_listing_names_descriptions_and_selection_semantics(strata, mode):
    root = strata.fixture.root.name
    pane = strata.pane(root)
    assert pane.name == root
    assert pane.description == f"{mode} view"

    container = strata.entry_container(root)
    assert container is not None
    assert container.name == root
    assert container.description == "Files"

    entries = strata.entries(root)
    assert [node.name for node in entries] == ROOT_ENTRIES
    for node in entries:
        expected = "Folder" if node.name in FOLDERS else "File"
        assert node.description == expected, (
            f"{node.name} should be described as a {expected}"
        )
        assert "focusable" in node.states, f"{node.name} should be focusable"

    # Exercise observable selection transitions instead of optional SELECTABLE state exports.
    strata.select_entry("todo.txt", directory=root)
    assert "selected" in strata.entry("todo.txt", directory=root).states
    others = [node for node in strata.entries(root) if node.name != "todo.txt"]
    assert all("selected" not in node.states for node in others)


def test_toolbar_controls_are_named(strata):
    for name in (
        "Search (Ctrl+K)",
        "Appearance",
        "Settings",
        "Close window",
        "Toggle sidebar (Ctrl+B)",
    ):
        assert strata.window.find(name=name) is not None, f"{name!r} is unnamed"


def test_focus_order_reaches_the_files_from_the_header(strata):
    """Tab from the window's first control eventually reaches the listing."""

    strata.keyboard.press("Tab")
    seen = []
    for _ in range(20):
        focused = strata.focused_node()
        if focused is None:
            strata.keyboard.press("Tab")
            continue
        seen.append(f"{focused.role}:{focused.name}")
        if focused.role in ("list", "table") or strata.focused_name() is not None:
            return
        strata.keyboard.press("Tab")
    raise AssertionError(f"Tab never reached the file listing; visited {seen}")


def _focus_outside(strata, surface):
    node = strata.focused_node()
    if node is None or not node.name or node == surface:
        return None
    if any(ancestor == surface for ancestor in node.ancestors()):
        return None
    return node


@pytest.mark.parametrize("entry", ["keyboard", "pointer"])
@pytest.mark.parametrize("mode", ALL_MODES)
def test_empty_directory_keeps_focus_and_tab_order(strata, mode, entry):
    strata.fixture.path("empty").mkdir()
    strata.keyboard.press("F5")
    strata.entry("empty")
    if entry == "keyboard":
        strata.select_entry_with_keyboard("empty")
        strata.keyboard.press("Return")
    elif mode == "Columns":
        strata.click_entry("empty")
    else:
        strata.double_click_entry("empty")
    strata.wait_for_directory("empty")

    surface = strata.wait(
        lambda: (node := strata.focused_node()) is not None and node.name == "empty" and node,
        "focus on the empty directory's pane surface",
        timeout=5,
    )
    assert surface.description == "This directory is empty"

    strata.keyboard.press("Tab")
    strata.wait(lambda: _focus_outside(strata, surface), "Tab to leave the empty pane", timeout=5)
    strata.keyboard.press("shift+Tab")
    strata.wait(
        lambda: strata.focused_node() == surface,
        "Shift+Tab to return to the empty pane",
        timeout=5,
    )
    strata.keyboard.press("shift+Tab")
    strata.wait(
        lambda: _focus_outside(strata, surface),
        "Shift+Tab to reach the control before the empty pane",
        timeout=5,
    )
