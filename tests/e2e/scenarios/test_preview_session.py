# SPDX-License-Identifier: MIT
import pytest
from PIL import Image

from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES


@pytest.fixture
def fixture_tree():
    tree = FixtureTree.create({
        "a.txt": "root preview\n",
        "z.zip": "unsupported fixture\n",
        "Alpha": {"a.txt": "alpha preview\n", "Beta": {"Gamma": {"Delta": {"a.txt": "delta preview\n"}}}},
    })
    Image.new("RGB", (80, 40), "green").save(tree.path("image.png"))
    try:
        yield tree
    finally:
        tree.cleanup()


def preview_option(strata):
    strata.open_appearance_menu()
    return strata.wait(
        lambda: strata.window.find(role="toggle button", name="Preview panel"),
        "the session preview toggle",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_preview_mode_survives_unsupported_selections_and_matches_appearance(strata, mode):
    strata.switch_view(mode)
    strata.select_entry("z.zip")
    option = preview_option(strata)
    assert option.find(role="label", name="Space") is not None
    assert not option.has_state("pressed")
    strata.pointer.click(option)
    if mode == "Icons":
        strata.wait(lambda: strata.preview_shows("No preview for this selection"), "the reserved preview space")
    else:
        assert strata.preview() is None
    option = preview_option(strata)
    assert option.has_state("pressed"), option.states
    strata.dismiss_menu()
    strata.select_entry("a.txt")
    strata.wait(lambda: strata.preview_shows("root preview"), "automatic preview after ZIP")
    strata.select_entry_with_keyboard("z.zip")
    if mode == "Icons":
        strata.wait(lambda: strata.preview_shows("No preview for this selection"), "the empty preview slot")
    else:
        strata.wait(lambda: strata.preview() is None, "unsupported selection to hide the panel")
    option = preview_option(strata)
    assert option.has_state("pressed"), option.states
    strata.pointer.click(option)
    strata.select_entry("a.txt")
    assert strata.preview() is None
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("root preview"), "Space to enable the panel")
    option = preview_option(strata)
    assert option.has_state("pressed"), option.states
    strata.dismiss_menu()
    strata.open_directory("Alpha")
    strata.select_entry_with_keyboard("a.txt")
    strata.wait(lambda: strata.preview_shows("alpha preview"), "preview after directory navigation")


@pytest.mark.preferences(browser_mode="icons", single_click_previews=False)
def test_icons_keep_their_layout_until_preview_mode_is_explicitly_toggled(strata):
    strata.select_entry("a.txt")
    full_width = strata.pane().screen_bounds().width
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("root preview"), "the initial preview")
    width = strata.pane().screen_bounds().width
    assert width < full_width

    def positions():
        return {name: strata.entry(name).screen_bounds() for name in ["Alpha", "a.txt", "image.png", "z.zip"]}

    layout = positions()

    def assert_layout():
        for name, actual in positions().items():
            expected = layout[name]
            assert abs(actual.x - expected.x) <= 1 and abs(actual.y - expected.y) <= 1, (name, expected, actual)
            assert actual.width == expected.width and actual.height == expected.height

    for name, expected in [("z.zip", "No preview for this selection"), ("image.png", "image/png"), ("Alpha", "No preview for this selection"), ("a.txt", "root preview")]:
        strata.select_entry_with_keyboard(name)
        strata.wait(lambda: strata.preview_shows(expected), f"the preview for {name}")
        assert strata.pane().screen_bounds().width == width
        assert_layout()
        if name in ["z.zip", "Alpha"]:
            assert not strata.preview_shows("root preview")
            assert strata.preview().find(role="label", name="a.txt") is None
            assert not strata.preview().find(role="button", name="Open in default application").has_state("sensitive")

    strata.pointer.click(strata.pane(), at=strata.background_point())
    strata.wait_for_selection([])
    strata.wait(lambda: strata.preview_shows("No preview for this selection"), "empty selection without reflow")
    assert_layout()
    strata.open_directory("Alpha")
    strata.wait(lambda: strata.preview_shows("No preview for this selection"), "the folder's preview slot")
    assert strata.pane().screen_bounds().width == width
    strata.select_entry("a.txt")
    strata.wait(lambda: strata.preview_shows("alpha preview"), "a preview inside the folder")
    assert strata.pane().screen_bounds().width == width
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is None and strata.pane().screen_bounds().width == full_width, "intentional toggle to restore the full grid")
    for name in ["Beta", "a.txt"]:
        strata.select_entry(name)
        assert strata.preview() is None
        assert strata.pane().screen_bounds().width == full_width


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_peek_click_reveals_a_column_without_activating_rows_or_toolbar_actions(strata):
    browser_left = strata.pane().screen_bounds().x
    for name in ["Alpha", "Beta", "Gamma", "Delta"]:
        strata.open_directory(name)
    def pane_count():
        return len(strata.window.find_all(description="Columns view", rendered=False))

    before = pane_count()
    peek = strata.wait(
        lambda: strata.window.find(role="button", name="Reveal Alpha column"),
        "a clickable parent-column peek",
    )
    bounds = peek.screen_bounds()
    heading = strata.pane("Alpha").find(role="label", name="Alpha", rendered=False)
    assert heading is not None
    strata.pointer.double_click(peek, at=(bounds.center[0], heading.screen_bounds().center[1]))
    strata.wait(lambda: strata.focused_name() == "Beta", "focus in the revealed parent")
    assert pane_count() == before
    strata.wait(
        lambda: strata.pane("Alpha").screen_bounds().x >= strata.sidebar_button("Home").screen_bounds().x,
        "the entire focused parent",
    )
    child = strata.wait(
        lambda: strata.window.find(role="button", name="Reveal Delta column"),
        "a clickable right-hand column peek",
    )
    strata.pointer.click(child)
    strata.wait(lambda: strata.pane("Delta").screen_bounds().x + strata.pane("Delta").screen_bounds().width <= strata.window_bounds().width, "the revealed child column")
    assert pane_count() == before
    parent = strata.wait(lambda: strata.window.find(role="button", name="Reveal Alpha column"), "the parent peek again")
    strata.pointer.click(parent)
    strata.wait(lambda: strata.focused_name() == "Beta", "the parent to receive focus")
    strata.select_entry("a.txt", directory="Alpha")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("alpha preview"), "the focused parent's preview")

    def focused_column_fits():
        column = strata.pane("Alpha").screen_bounds()
        return column.x >= browser_left and column.x + column.width <= strata.preview().screen_bounds().x

    strata.wait(focused_column_fits, "the focused parent, not the rightmost column, to remain visible")
    assert pane_count() == before
