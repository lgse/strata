# SPDX-License-Identifier: MIT
"""Golden screenshots for a small set of deliberately stable states.

Interaction assertions are the primary gate; these catch rendering
regressions the accessible tree cannot see. Keep the set small: every image
here has to be reviewed whenever the design changes.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from PIL import Image

from harness.fixtures import FixtureTree

BASELINE_FIXTURE = {
    "documents": {
        "notes.txt": "notes\n",
        "projects": {"release": {"summary.md": "# Release\n"}},
        "report.md": "# Report\n",
    },
    "pictures": {},
    "readme.md": "# Fixture\n",
    "todo.txt": "todo\n",
}

pytestmark = pytest.mark.baseline


# The breadcrumb and the context menu both render the fixture's path, so the
# baseline scenarios use a fixed directory instead of a randomized one.
BASELINE_ROOT = Path("/tmp/strata-e2e-baseline")
BASELINE_ENTRIES = ["documents", "pictures", "readme.md", "todo.txt"]
ICONS_BASELINE_FIXTURE = {
    "Applications": {},
    "DataGripProjects": {},
    "pictures": {},
    "readme.md": "# Fixture\n",
    "todo.txt": "todo\n",
}
ICONS_BASELINE_ENTRIES = [
    "Applications", "DataGripProjects", "pictures", "portrait.png", "readme.md",
    "todo.txt", "wide.png",
]
ICONS_BASELINE_THUMBNAILS = {
    "portrait.png": ((32, 64), (255, 140, 0)),
    "wide.png": ((64, 32), (70, 130, 180)),
}


@pytest.fixture
def fixture_tree(request):
    """Stable names and content, including mixed caption lengths and image shapes."""

    preferences = request.node.get_closest_marker("preferences")
    icons = preferences is not None and preferences.kwargs.get("browser_mode") == "icons"
    tree = FixtureTree.create_at(
        BASELINE_ROOT, ICONS_BASELINE_FIXTURE if icons else BASELINE_FIXTURE
    )
    if icons:
        for name, (size, color) in ICONS_BASELINE_THUMBNAILS.items():
            Image.new("RGB", size, color).save(BASELINE_ROOT / name)
    try:
        yield tree
    finally:
        tree.cleanup()


@pytest.mark.preferences(browser_mode="columns")
def test_columns_view_baseline(strata, baseline):
    _settle(strata)
    baseline(strata, "columns-view")


@pytest.mark.preferences(browser_mode="columns")
def test_columns_overflow_baseline(strata, baseline, tmp_path):
    strata.open_directory("documents")
    strata.open_directory("projects", "documents")
    strata.open_directory("release", "projects")
    _settle(strata, ["summary.md"])

    capture = strata.screenshot(tmp_path / "columns-overflow.png")
    sidebar = strata.sidebar_button("Home").parent
    assert sidebar is not None
    sidebar_bounds = sidebar.screen_bounds()
    pane_bounds = strata.pane("release").screen_bounds()
    leading_edge = sidebar_bounds.x + sidebar_bounds.width
    scrollbar_y = pane_bounds.y + pane_bounds.height + 7
    with Image.open(capture) as image:
        pixels = image.convert("RGB")
        assert pixels.getpixel((leading_edge, scrollbar_y)) == pixels.getpixel(
            (leading_edge + 20, scrollbar_y)
        ), "the horizontal scrollbar background should be continuous at its leading edge"

    baseline(strata, "columns-overflow")


@pytest.mark.preferences(browser_mode="icons")
def test_icons_view_baseline(strata, baseline, tmp_path):
    _settle_icons(strata, tmp_path)
    strata.select_entry_with_keyboard("DataGripProjects")
    baseline(strata, "icons-view")


@pytest.mark.preferences(browser_mode="icons", browser_density="airy")
def test_icons_airy_view_baseline(strata, baseline, tmp_path):
    _settle_icons(strata, tmp_path)
    strata.select_entry_with_keyboard("Applications")
    baseline(strata, "icons-airy-view")


@pytest.mark.preferences(browser_mode="icons")
def test_icons_hover_baseline(strata, baseline, tmp_path):
    _settle_icons(strata, tmp_path)
    strata.pointer.move_to(*strata.entry("todo.txt").screen_bounds().center)
    strata.settle(strata.pane())
    baseline(strata, "icons-hover")


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


def _settle_icons(strata, tmp_path) -> None:
    _settle(strata, ICONS_BASELINE_ENTRIES)
    icons = {
        name: strata.entry(name).find(role="image")
        for name in ICONS_BASELINE_THUMBNAILS
    }
    assert all(icon is not None for icon in icons.values())

    def thumbnails_ready():
        capture = strata.screenshot(tmp_path / "thumbnail-readiness.png")
        with Image.open(capture) as image:
            pixels = image.convert("RGB")
            return all(
                pixels.getpixel(icons[name].screen_bounds().center) == color
                for name, (_, color) in ICONS_BASELINE_THUMBNAILS.items()
            )

    strata.wait(thumbnails_ready, "both image shapes to finish rendering")


def _settle(strata, entries=BASELINE_ENTRIES) -> None:
    """Park the pointer and wait for the listing before capturing."""

    strata.park_pointer()
    strata.wait(
        lambda: strata.entry_names()
        == entries,
        "the fixture listing to be complete",
    )
    strata.settle(strata.pane())
