# SPDX-License-Identifier: GPL-3.0-or-later
"""Real-input regression coverage for revealing a renamed entry."""

import os
from pathlib import Path

import pytest

from harness.modes import ALL_MODES


def fully_in_pane(node, pane_bounds) -> bool:
    bounds = node.screen_bounds()
    return (
        bounds.width > 0
        and bounds.height > 0
        and bounds.x >= pane_bounds.x
        and bounds.y >= pane_bounds.y
        and bounds.x + bounds.width <= pane_bounds.x + pane_bounds.width
        and bounds.y + bounds.height <= pane_bounds.y + pane_bounds.height
    )


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", ("file", "folder"))
def test_new_entries_focus_and_reveal_the_editor_in_a_long_listing(strata, mode, kind):
    strata.switch_view(mode)
    for index in range(160):
        strata.fixture.path(f"b-entry-{index:03d}").mkdir(exist_ok=True)

    strata.keyboard.press("F5")
    strata.entry("b-entry-000")
    strata.keyboard.press("Home")

    if kind == "file":
        # The permanent list gutter is outside virtualized rows, unlike the
        # bottom edge used by background_point when the viewport is full.
        strata.pointer.right_click(strata.pane(), at=strata.folder_context_point())
        strata.choose_menu_item("New File")
    else:
        strata.keyboard.press("ctrl+shift+n")
    field = strata.editable_field()
    original = "new " + kind
    strata.wait(lambda: field.text == original, f"the default new-{kind} name")
    strata.wait(strata.fixture.path(original).exists, f"the new {kind} on disk")
    pane_bounds = strata.containers()[-1].screen_bounds()
    strata.wait(
        lambda: fully_in_pane(field, pane_bounds),
        f"the new {kind} rename editor to be fully visible",
    )
    strata.keyboard.type_text("typed-name")
    strata.wait(lambda: field.text == "typed-name", "typing to replace the selected default name")
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable"}) is None,
        f"the {kind} rename editor to close",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_rename_to_opposite_sorted_edge_reveals_selected_entry(strata, mode):
    strata.switch_view(mode)
    original = "new folder"
    renamed = "a-final"
    for index in range(80):
        strata.fixture.path(f"b-entry-{index:03d}").mkdir()
    for index in range(3):
        strata.fixture.path(f"z-tail-{index:03d}").mkdir()
    # Ctrl+Shift+N is the existing all-mode creation path for folders and does
    # not depend on a background point remaining empty after virtualization.
    strata.keyboard.press("ctrl+shift+n")
    field = strata.wait(
        lambda: strata.window.find(
            role="text", name="Rename", states={"editable", "focused"}
        ),
        "the new-entry rename editor to take focus",
    )
    strata.wait(lambda: field.text == original, "the default new-folder name")
    strata.wait(strata.fixture.path(original).exists, "the new folder on disk")
    initial = strata.entry(original)
    initial_bounds = initial.screen_bounds()
    pane_bounds = strata.containers()[-1].screen_bounds()
    strata.wait(
        lambda: fully_in_pane(field, pane_bounds),
        "the initial rename editor to remain fully visible",
    )
    capture_evidence(strata, "before")

    strata.keyboard.type_text(renamed)
    strata.wait(lambda: field.text == renamed, "the replacement name")
    strata.keyboard.press("Return")

    destination = strata.fixture.path(renamed)
    strata.wait(destination.exists, "the renamed entry on disk")
    strata.wait_for_entry_gone(original)
    strata.wait_for_selection([renamed])
    strata.wait(
        lambda: fully_in_pane(strata.entry(renamed), strata.containers()[-1].screen_bounds()),
        "the renamed folder to be revealed fully in the pane",
    )
    assert strata.entry(renamed).screen_bounds() != initial_bounds
    capture_evidence(strata, "after")
    assert destination.is_dir()


def capture_evidence(strata, label: str) -> None:
    directory = os.environ.get("STRATA_RENAME_EVIDENCE_DIR")
    if directory:
        strata.screenshot(Path(directory) / f"{label}.png")
