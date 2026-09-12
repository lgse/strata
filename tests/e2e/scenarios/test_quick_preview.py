# SPDX-License-Identifier: MIT
"""Opening, reading, and closing the quick preview."""

from __future__ import annotations

import pytest

from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES, NEXT_ENTRY_KEY, PREVIOUS_ENTRY_KEY

PREVIEW_FIXTURE = {
    "notes.txt": "the quick brown fox\n",
    "page.md": "# Heading\n\nBody text.\n",
    "third.txt": "third preview fixture\n",
    "data.csv": "name,value\nalpha,1\n",
    "folder": {"inner.txt": "inner\n", "nested-notes.txt": "nested preview fixture\n"},
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
    before = strata.entry_names()
    strata.select_entry_with_keyboard("notes.txt")

    strata.keyboard.press("space")

    strata.wait(
        lambda: strata.preview_shows("the quick brown fox"),
        "the preview to render the file's text",
    )

    preview = strata.preview()
    assert preview.find(role="label", name="notes.txt") is not None
    assert strata.preview_shows("text/plain")
    assert strata.entry_names() == before
    close = preview.find(role="button", name="Close preview (Space)")
    strata.pointer.click(close)
    strata.wait(lambda: strata.preview() is None, "the preview to close")


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("selection", ["keyboard", "pointer"])
def test_space_previews_a_filtered_result_without_changing_the_query(strata, mode, selection):
    strata.select_entry("notes.txt")
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text("nested-notes")
    strata.wait(
        lambda: strata.matches() == ["nested-notes.txt"],
        "the nested search result",
    )
    if selection == "keyboard":
        strata.keyboard.press("Down")
    else:
        result = strata.window.find(name="nested-notes.txt", role="list item")
        assert result is not None
        strata.pointer.click(result, modifiers=("ctrl",))

    strata.keyboard.press("space")
    strata.wait(
        lambda: strata.preview_shows("nested preview fixture"),
        "Space to preview the search result, not the stale directory selection",
    )
    assert field.text == "nested-notes"
    assert strata.matches() == ["nested-notes.txt"]
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is None, "Space to close the filtered preview")
    assert field.text == "nested-notes"
    assert strata.matches() == ["nested-notes.txt"]


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("selection", ["keyboard", "pointer"])
@pytest.mark.parametrize(
    "preferences",
    [{"single_click_previews": False}, {"single_click_previews": True}],
    ids=["explicit-preview", "single-click-preview"],
)
def test_preview_follows_the_selection(strata, mode, selection, preferences):
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(
        lambda: strata.preview_shows("the quick brown fox"),
        "the first preview to render",
    )

    if selection == "keyboard":
        strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    else:
        strata.select_entry("page.md")
    strata.wait_for_selection(["page.md"])

    strata.wait(
        lambda: strata.preview_shows("Body text."),
        "the preview to follow the newly selected file",
    )
    assert strata.focused_name() == "page.md"
    assert not strata.preview_shows("the quick brown fox")


def test_list_preview_keyboard_navigation_preserves_horizontal_scroll(strata, fixture_tree):
    strata.switch_view("List")
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the first preview")

    def list_scroll_origin():
        container = strata.pane()
        list_view = container.find(description="Files")
        assert list_view is not None, "the List file view"
        panes = [
            ancestor for ancestor in list_view.ancestors() if ancestor.role == "scroll pane"
        ]
        assert len(panes) >= 2, "the List view content is wrapped by its listing scroller"
        return panes[0].window_bounds().x

    origin = list_scroll_origin()
    for key, name, preview in (
        ("Down", "page.md", "Body text."),
        ("Down", "third.txt", "third preview fixture"),
        ("Up", "page.md", "Body text."),
    ):
        strata.keyboard.press(key)
        strata.wait_for_selection([name])
        strata.wait(lambda: strata.focused_name() == name, f"focus on {name}")
        strata.wait(lambda: strata.preview_shows(preview), f"preview for {name}")
        assert list_scroll_origin() == origin, (origin, list_scroll_origin())


@pytest.mark.parametrize("mode", ALL_MODES)
def test_preview_follows_extended_selection_without_collapsing_it(strata, mode):
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the first preview")

    strata.keyboard.press(f"shift+{NEXT_ENTRY_KEY[mode]}")

    strata.wait_for_selection(["notes.txt", "page.md"])
    strata.wait(lambda: strata.preview_shows("Body text."), "the newly focused preview")
    assert strata.focused_name() == "page.md"


@pytest.mark.parametrize("mode", ALL_MODES)
def test_preview_hides_on_a_folder_and_resumes_when_selection_moves(strata, mode):
    strata.select_entry_with_keyboard("data.csv")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("alpha"), "the file preview")

    strata.keyboard.press(PREVIOUS_ENTRY_KEY[mode])

    strata.wait_for_selection(["folder"])
    strata.wait(lambda: strata.preview() is None, "the folder to dismiss the preview")
    strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    strata.wait_for_selection(["data.csv"])
    strata.wait(lambda: strata.preview_shows("alpha"), "the still-enabled preview to resume")


def test_preview_hides_on_shift_range_folder_focus(strata):
    strata.switch_view("List")
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the file preview")

    strata.keyboard.press("shift+Up")
    strata.wait_for_selection(["data.csv", "notes.txt"])
    strata.wait(lambda: strata.focused_name() == "data.csv", "the focused upper file")
    strata.wait(lambda: strata.preview_shows("alpha"), "the upper file preview")

    strata.keyboard.press("shift+Up")
    strata.wait_for_selection(["folder", "data.csv", "notes.txt"])
    strata.wait(lambda: strata.focused_name() == "folder", "the focused folder")
    strata.wait(lambda: strata.preview() is None, "the focused folder to dismiss the preview")

    strata.keyboard.press("shift+Down")
    strata.wait_for_selection(["data.csv", "notes.txt"])
    strata.wait(lambda: strata.preview_shows("alpha"), "preview to resume after the folder")


def test_preview_renders_markdown(strata):
    strata.select_entry_with_keyboard("page.md")
    strata.keyboard.press("space")

    strata.wait(
        lambda: strata.preview_shows("Body text."),
        "the markdown preview to render its body",
    )


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_column_preview_fills_free_space_and_remembers_a_dragged_session_width(strata):
    def adjacent():
        column = strata.containers()[-1].screen_bounds()
        preview = strata.preview().screen_bounds()
        return abs(preview.x - (column.x + column.width)) <= 3

    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the first preview")
    strata.wait(adjacent, "the preview to meet the last column")
    initial = strata.preview().screen_bounds().width
    strata.keyboard.press("Down")
    strata.wait(lambda: strata.preview_shows("Body text."), "keyboard selection to update the preview")
    strata.keyboard.press("space")
    strata.open_directory("folder")
    strata.select_entry_with_keyboard("inner.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("inner"), "the nested preview")
    strata.wait(adjacent, "columns to scroll left beside the minimum-width preview")
    minimum = strata.preview().screen_bounds().width
    assert minimum < initial
    assert strata.containers()[0].screen_bounds().x < strata.pane("folder").screen_bounds().x

    bounds = strata.preview().screen_bounds()
    start = (bounds.x - 1, bounds.y + bounds.height // 2)
    distance = bounds.width // 5
    strata.pointer.drag_points(start, (start[0] + distance, start[1]))
    strata.wait(
        lambda: strata.preview().screen_bounds().width < minimum - distance // 2,
        "the dragged width to override the automatic minimum",
    )
    chosen = strata.preview().screen_bounds().width
    resized = strata.preview().screen_bounds()
    window = strata.window_bounds()
    assert abs(resized.x + resized.width - window.x - window.width) <= 2
    strata.keyboard.press("space")
    strata.select_entry_with_keyboard("nested-notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("nested preview fixture"), "the reopened preview")
    assert abs(strata.preview().screen_bounds().width - chosen) <= 2
    strata.keyboard.press("space")
    strata.keyboard.press("alt+Left")
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the parent preview")
    assert abs(strata.preview().screen_bounds().width - chosen) <= 2


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_closing_preview_does_not_move_the_columns(strata):
    strata.open_directory("folder")
    strata.select_entry_with_keyboard("inner.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("inner"), "the nested preview")
    column = strata.pane("folder")
    scroller = next(node for node in column.ancestors() if node.role == "scroll pane")
    before = column.screen_bounds()
    viewport_width = scroller.screen_bounds().width
    close = strata.preview().find(role="button", name="Close preview (Space)")
    strata.pointer.click(close)
    strata.wait(lambda: strata.preview() is None, "the preview to close")
    strata.wait(lambda: scroller.screen_bounds().width > viewport_width, "the browser to use the released space")
    assert abs(strata.pane("folder").screen_bounds().x - before.x) <= 1
    strata.select_entry("inner.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("inner"), "the preview to reopen")
    strata.wait(
        lambda: abs(strata.pane("folder").screen_bounds().x + before.width - strata.preview().screen_bounds().x) <= 3,
        "the reopened preview to meet the last column",
    )


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_narrow_window_prioritizes_the_last_column_and_restores_the_latest_preview(strata):
    browser_left = strata.pane().screen_bounds().x
    strata.open_directory("folder")
    strata.select_entry_with_keyboard("inner.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("inner"), "the initial preview")
    preferred = strata.preview().screen_bounds().width

    def resize(width):
        bounds = strata.window_bounds()
        strata.keyboard.connection.resize_surface(bounds.width, bounds.height, width, bounds.height)
        strata.wait(lambda: strata.window_bounds().width == width, "the resized window")

    def last_column_visible():
        column = strata.pane("folder").screen_bounds()
        window = strata.window_bounds()
        return column.x >= browser_left and column.x + column.width <= window.x + window.width

    resize(900)
    strata.wait(lambda: strata.preview().screen_bounds().width < preferred, "the preview minimum to yield")
    strata.wait(last_column_visible, "the entire last column to stay visible")
    column = strata.pane("folder").screen_bounds()
    assert column.x + column.width <= strata.preview().screen_bounds().x
    resize(760)
    strata.wait(lambda: strata.preview() is None, "the unusably narrow preview to hide")
    strata.wait(last_column_visible, "the last column without the preview")
    strata.keyboard.press("Down")
    strata.wait_for_selection(["nested-notes.txt"])
    assert strata.preview() is None
    resize(900)
    strata.wait(lambda: strata.preview_shows("nested preview fixture"), "the latest selection to resume")
    strata.wait(last_column_visible, "the last column beside the resumed preview")
    resize(1200)
    strata.wait(lambda: strata.preview().screen_bounds().width == preferred, "the preferred preview width to return")


def test_space_opens_the_preview_after_a_pointer_selection(strata):
    strata.select_entry("notes.txt")

    strata.keyboard.press("space")

    strata.wait(strata.preview, "the preview to open on the first Space")
