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
    pane_bounds = strata.pane().screen_bounds()
    strata.wait(
        lambda: fully_in_pane(strata.entry(original), pane_bounds),
        "the initial new folder to remain fully visible",
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
        lambda: fully_in_pane(strata.entry(renamed), strata.pane().screen_bounds()),
        "the renamed folder to be revealed fully in the pane",
    )
    assert strata.entry(renamed).screen_bounds() != initial_bounds
    capture_evidence(strata, "after")
    assert destination.is_dir()


def capture_evidence(strata, label: str) -> None:
    directory = os.environ.get("STRATA_RENAME_EVIDENCE_DIR")
    if directory:
        strata.screenshot(Path(directory) / f"{label}.png")
