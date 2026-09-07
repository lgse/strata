# SPDX-License-Identifier: GPL-3.0-or-later
"""Golden screenshots for a small set of deliberately stable states.

Interaction assertions are the primary gate; these catch rendering
regressions the accessible tree cannot see. Keep the set small: every image
here has to be reviewed whenever the design changes.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from harness.fixtures import FixtureTree

BASELINE_FIXTURE = {
    "documents": {"notes.txt": "notes\n", "report.md": "# Report\n"},
    "pictures": {},
    "readme.md": "# Fixture\n",
    "todo.txt": "todo\n",
}

pytestmark = pytest.mark.baseline


# The breadcrumb and the context menu both render the fixture's path, so the
# baseline scenarios use a fixed directory instead of a randomized one.
BASELINE_ROOT = Path("/tmp/strata-e2e-baseline")
BASELINE_ENTRIES = ["documents", "pictures", "readme.md", "todo.txt"]


@pytest.fixture
def fixture_tree():
    """A fixture whose path, names, and sizes never change."""

    tree = FixtureTree.create_at(BASELINE_ROOT, BASELINE_FIXTURE)
    try:
        yield tree
    finally:
        tree.cleanup()


@pytest.mark.preferences(browser_mode="columns")
def test_columns_view_baseline(strata, baseline):
    _settle(strata)
    baseline(strata, "columns-view")


@pytest.mark.preferences(browser_mode="icons")
def test_icons_view_baseline(strata, baseline):
    _settle(strata)
    baseline(strata, "icons-view")


@pytest.mark.preferences(browser_mode="list")
def test_list_view_baseline(strata, baseline):
    _settle(strata)
    baseline(strata, "list-view")


@pytest.mark.preferences(browser_mode="list")
def test_selection_and_focus_baseline(strata, baseline):
    strata.select_entry_with_keyboard("readme.md")
    strata.keyboard.press("shift+Down")
    strata.wait(
        lambda: strata.selected_names() == ["readme.md", "todo.txt"],
        "both files to be selected",
    )
    _settle(strata)
    baseline(strata, "selection-and-focus")


@pytest.mark.preferences(browser_mode="list")
def test_context_menu_baseline(strata, baseline):
    strata.open_context_menu("readme.md")
    strata.wait(
        lambda: "Copy" in strata.menu_items(), "the context menu to be populated"
    )
    baseline(strata, "context-menu")


@pytest.mark.preferences(browser_mode="list")
def test_delete_confirmation_baseline(strata, baseline):
    strata.select_entry_with_keyboard("todo.txt")
    strata.keyboard.press("shift+Delete")
    strata.wait_for_dialog()
    _settle(strata)
    baseline(strata, "delete-confirmation")


def _settle(strata) -> None:
    """Park the pointer and wait for the listing before capturing."""

    strata.park_pointer()
    strata.wait(
        lambda: strata.entry_names()
        == BASELINE_ENTRIES,
        "the fixture listing to be complete",
    )
    strata.settle(strata.pane())
