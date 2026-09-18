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

    # GTK 4.14 omits SELECTABLE on unselected rows; exercise SELECTED transitions.
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


def test_location_controls_are_named(strata):
    """`ui::accessibility::describe_location_controls` names the address bar."""

    strata.keyboard.press("ctrl+l")
    field = strata.wait(
        lambda: strata.window.find(role="text", name="Location (Ctrl+L)"),
        "the named location entry",
    )
    assert "editable" in field.states
    for name in ("Navigate (Enter)", "Cancel (Escape)"):
        assert strata.window.find(role="button", name=name) is not None, (
            f"{name!r} is unnamed"
        )


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
