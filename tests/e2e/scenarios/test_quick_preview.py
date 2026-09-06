# SPDX-License-Identifier: GPL-3.0-or-later
"""Opening, reading, and closing the quick preview."""

from __future__ import annotations

import pytest

from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES
from harness.tree import TreeTimeout

PREVIEW_FIXTURE = {
    "notes.txt": "the quick brown fox\n",
    "page.md": "# Heading\n\nBody text.\n",
    "data.csv": "name,value\nalpha,1\n",
    "folder": {"inner.txt": "inner\n"},
}


@pytest.fixture
def fixture_tree():
    """Replaces the shared fixture with file types the preview can render."""

    tree = FixtureTree.create(PREVIEW_FIXTURE)
    try:
        yield tree
    finally:
        tree.cleanup()


@pytest.mark.parametrize("mode", ALL_MODES)
def test_space_opens_and_closes_the_quick_preview(strata, mode):
    strata.select_entry_with_keyboard("notes.txt")

    strata.keyboard.press("space")

    strata.wait(
        lambda: strata.preview_shows("the quick brown fox"),
        "the preview to render the file's text",
    )

    close = strata.preview().find(role="button", name="Close preview (Space)")
    strata.pointer.click(close)
    strata.wait(lambda: strata.preview() is None, "the preview to close")


def test_preview_follows_the_selection(strata):
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(
        lambda: strata.preview_shows("the quick brown fox"),
        "the first preview to render",
    )

    strata.select_entry_with_keyboard("data.csv")

    strata.wait(
        lambda: strata.preview_shows("alpha"),
        "the preview to follow the newly selected file",
    )


def test_preview_renders_markdown(strata):
    strata.select_entry_with_keyboard("page.md")
    strata.keyboard.press("space")

    strata.wait(
        lambda: strata.preview_shows("Body text."),
        "the markdown preview to render its body",
    )


def test_the_preview_reports_the_file_it_is_showing(strata):
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")

    preview = strata.wait(strata.preview, "the preview to open")
    assert any(
        node.name == "notes.txt" for node in preview.find_all(role="label")
    ), "the preview should name the file it is showing"
    assert strata.preview_shows("text/plain"), (
        "the preview should report the file's type"
    )


def test_opening_the_preview_leaves_the_listing_intact(strata):
    before = strata.entry_names()

    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(strata.preview, "the preview to open")

    assert strata.entry_names() == before, (
        "opening the preview must not disturb the directory listing"
    )


@pytest.mark.xfail(
    strict=True,
    raises=TreeTimeout,
    reason="Pointer selection still needs a second Space before quick preview opens.",
)
def test_space_opens_the_preview_after_a_pointer_selection(strata):
    strata.select_entry("notes.txt")

    strata.keyboard.press("space")

    strata.wait(strata.preview, "the preview to open on the first Space")
