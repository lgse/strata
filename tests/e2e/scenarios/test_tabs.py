# SPDX-License-Identifier: MIT
"""Live browser tabs, keyboard routing and cross-tab file transfers."""

import pytest
from gi.repository import Atspi

from harness.interaction import MODIFIER_KEYSYMS
from harness.modes import ALL_MODES


def tab(strata, name):
    return strata.wait(
        lambda: strata.window.find(role="page tab", name=name),
        f"tab {name}",
    )


def selected_tab(strata, name):
    return strata.wait(
        lambda: tab(strata, name).has_state("selected"),
        f"active tab {name}",
    )


@pytest.mark.preferences(browser_mode="columns")
def test_folder_click_keeps_tab_name_until_release_and_keyboard_focus_still_renames(strata):
    parent = "documents"
    strata.fixture.path("documents/alpha").mkdir()
    strata.fixture.path("documents/beta").mkdir()
    strata.keyboard.press("ctrl+t")
    strata.keyboard.press("ctrl+l")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(strata.fixture.path(parent)))
    strata.keyboard.press("Return")
    strata.wait_for_directory(parent)
    strata.open_directory("alpha")
    selected_tab(strata, "alpha")
    scrollbar = strata.wait(
        lambda: strata.window.find(role="scroll bar", states={"horizontal"}),
        "column scrollbar",
    )
    value = Atspi.Accessible.get_value_iface(scrollbar.accessible)
    assert Atspi.Value.set_current_value(value, 0.0)
    target = strata.settle(strata.entry("beta", directory=parent))
    origin = strata.pointer.drag_origin(target)
    try:
        strata.pointer.drag_points(origin, origin, release=False)
        strata.settle(target)
        strata.wait_for_selection(["beta"], parent)
        assert strata.window.find(role="page tab", name="alpha", states={"selected"}) is not None
        assert strata.window.find(role="page tab", name="beta") is None
    finally:
        strata.pointer.connection.button(1, False)
    selected_tab(strata, "beta")
    strata.wait_for_directory("beta")
    strata.wait_for_selection([], "beta")
    strata.keyboard.press("Left")
    strata.wait(
        lambda: strata.window.find(role="page tab", name=parent, states={"selected"}),
        "parent tab name after deliberate keyboard focus",
    )
    strata.keyboard.press("Right")
    selected_tab(strata, "beta")


@pytest.mark.parametrize("tenxer", [
    pytest.param(False, marks=pytest.mark.preferences(tenxer_mode=False)),
    pytest.param(True, marks=pytest.mark.preferences(tenxer_mode=True)),
])
def test_tabs_keep_locations_and_support_numbered_shortcuts(strata, tenxer):
    root = strata.fixture.root.name
    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+t")
    strata.open_directory("archive")
    selected_tab(strata, "archive")
    ctrl, shift = MODIFIER_KEYSYMS["ctrl"], MODIFIER_KEYSYMS["shift"]
    strata.keyboard.connection.key(ctrl, True)
    strata.keyboard.connection.key(shift, True)
    try:
        strata.wait(
            lambda: strata.window.find(role="label", name="1"),
            "tab number hints while Ctrl+Shift is held",
        )
    finally:
        strata.keyboard.connection.key(shift, False)
        strata.keyboard.connection.key(ctrl, False)
    strata.keyboard.press("ctrl+shift+1")
    selected_tab(strata, root)
    strata.wait_for_selection(["todo.txt"], root)
    strata.keyboard.press("ctrl+Tab")
    selected_tab(strata, "archive")
    strata.keyboard.press("ctrl+shift+Tab")
    selected_tab(strata, root)
    strata.keyboard.press("ctrl+shift+2")
    selected_tab(strata, "archive")
    strata.keyboard.press("ctrl+t")
    strata.keyboard.press("alt+Up")
    strata.wait_for_directory(root)
    strata.open_directory("pictures")
    selected_tab(strata, "pictures")
    for shortcut, names in [
        ("ctrl+Page_Up", ["archive", root, "pictures"]),
        ("ctrl+Page_Down", [root, "archive", "pictures"]),
    ]:
        strata.keyboard.press("ctrl+l")
        strata.wait(
            lambda: strata.window.find(role="text", name="Location (Ctrl+L)", states={"focused"}),
            "location editor to take focus",
        )
        for name in names:
            strata.keyboard.press(shortcut)
            selected_tab(strata, name)
    for shortcut, order in [
        ("ctrl+shift+Page_Up", [root, "pictures", "archive"]),
        ("ctrl+shift+Page_Up", ["pictures", root, "archive"]),
        ("ctrl+shift+Page_Up", ["pictures", root, "archive"]),
        ("ctrl+shift+Page_Down", [root, "pictures", "archive"]),
        ("ctrl+shift+Page_Down", [root, "archive", "pictures"]),
        ("ctrl+shift+Page_Down", [root, "archive", "pictures"]),
    ]:
        strata.keyboard.press("ctrl+l")
        field = strata.wait(
            lambda: strata.window.find(role="text", name="Location (Ctrl+L)", states={"focused"}),
            "location editor to take focus",
        )
        strata.keyboard.press(shortcut)
        selected_tab(strata, "pictures")
        strata.wait(lambda: field.has_state("focused"), "reordering to preserve location editor focus")
        strata.keyboard.press("Escape")
        for index, name in enumerate(order, start=1):
            strata.keyboard.press(f"ctrl+shift+{index}")
            selected_tab(strata, name)
        strata.keyboard.press(f"ctrl+shift+{order.index('pictures') + 1}")
        selected_tab(strata, "pictures")
    strata.keyboard.press("ctrl+w")
    selected_tab(strata, "archive")
    strata.keyboard.press("ctrl+w")
    strata.wait_for_selection(["todo.txt"], root)
    # The same add control works after the strip collapses.
    strata.pointer.click(strata.wait(lambda: strata.window.find(role="button", name="New tab"), "New tab"))
    strata.open_directory("documents")
    selected_tab(strata, "documents")
    assert strata.environment.read_preferences().get("tenxer_mode") == str(tenxer).lower()


@pytest.mark.parametrize("mode", ALL_MODES)
def test_drop_on_another_tab_moves_files_into_its_location(strata, mode):
    root = strata.fixture.root.name
    strata.keyboard.press("ctrl+t")
    strata.open_directory("archive")
    selected_tab(strata, "archive")
    strata.keyboard.press("ctrl+shift+1")
    selected_tab(strata, root)
    source = strata.select_entry("todo.txt")
    strata.pointer.drag(source, tab(strata, "archive"))
    strata.wait(lambda: strata.fixture.path("archive/todo.txt").exists(), "file transferred to the other tab")
    assert not strata.fixture.path("todo.txt").exists()
    assert strata.fixture.path("archive/todo.txt").read_text() == "todo\n"


def test_hovering_a_tab_during_drag_allows_a_drop_in_its_listing(strata):
    strata.fixture.path("documents/projects").mkdir()
    root = strata.fixture.root.name
    strata.keyboard.press("ctrl+t")
    strata.open_directory("documents")
    selected_tab(strata, "documents")
    strata.keyboard.press("ctrl+shift+1")
    selected_tab(strata, root)
    source = strata.select_entry("todo.txt")
    strata.pointer.drag_points(
        strata.pointer.drag_origin(source),
        tab(strata, "documents").screen_bounds().center,
        release=False,
    )
    try:
        selected_tab(strata, "documents")
        target = strata.entry("projects")
        # A drag icon can match a row's native-surface size. Avoid the legacy
        # popup-origin correction while the drag surface is alive.
        x, y = target.window_bounds().center
        origin = strata.window.screen_bounds()
        strata.pointer.move_to(origin.x + x, origin.y + y)
    finally:
        strata.pointer.connection.button(1, False)
    strata.wait(lambda: strata.fixture.path("documents/projects/todo.txt").exists(), "file dropped after switching tabs during drag")
    assert not strata.fixture.path("todo.txt").exists()


def test_dragging_tab_labels_changes_numbered_order(strata):
    root = strata.fixture.root.name
    strata.keyboard.press("ctrl+t")
    strata.open_directory("archive")
    selected_tab(strata, "archive")
    strata.pointer.drag(tab(strata, "archive"), tab(strata, root))
    strata.keyboard.press("ctrl+shift+2")
    selected_tab(strata, root)
    strata.keyboard.press("ctrl+shift+1")
    selected_tab(strata, "archive")
    strata.keyboard.press("ctrl+Page_Down")
    selected_tab(strata, root)
    strata.keyboard.press("ctrl+Page_Up")
    selected_tab(strata, "archive")


def tab_chord(strata, suffix):
    strata.keyboard.press("t")
    strata.keyboard.press(suffix)


@pytest.mark.preferences(tenxer_mode=True)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_tenxer_tab_chords_create_select_close_and_respect_text_input(strata, mode):
    root = strata.fixture.root.name
    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+l")
    field = strata.wait(lambda: strata.window.find(role="text", name="Location (Ctrl+L)"), "location editor")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("tntt")
    strata.wait(lambda: field.text == "tntt", "tab chord letters to remain literal text")
    strata.keyboard.press("Escape")
    strata.keyboard.press("t")
    strata.wait(lambda: strata.window.find(role="label", name="t-"), "tab chord options")
    strata.keyboard.press("Escape")
    strata.keyboard.press("n")
    strata.wait_for_selection(["todo.txt"], root)
    tab_chord(strata, "n")
    strata.open_directory("archive")
    selected_tab(strata, "archive")
    tab_chord(strata, "1")
    selected_tab(strata, root)
    strata.wait_for_selection(["todo.txt"], root)
    tab_chord(strata, "2")
    selected_tab(strata, "archive")
    tab_chord(strata, "n")
    strata.keyboard.press("alt+Up")
    strata.wait_for_directory(root)
    strata.open_directory("pictures")
    selected_tab(strata, "pictures")
    tab_chord(strata, "t")
    selected_tab(strata, "archive")
    tab_chord(strata, "t")
    selected_tab(strata, root)
    strata.wait_for_selection(["todo.txt"], root)
    tab_chord(strata, "t")
    selected_tab(strata, "pictures")
    tab_chord(strata, "x")
    selected_tab(strata, "archive")
    tab_chord(strata, "x")
    tab_chord(strata, "t")
    strata.wait_for_selection(["todo.txt"], root)


@pytest.mark.preferences(tenxer_mode=True)
def test_zero_selects_the_tenth_tab_with_chords_and_numbered_shortcuts(strata):
    strata.entry("todo.txt")
    for number in range(2, 11):
        strata.fixture.path(f"tab-{number}").mkdir()
    strata.keyboard.press("F5")
    strata.entry("tab-10")
    root = strata.fixture.root.name
    for number in range(2, 11):
        strata.keyboard.press("ctrl+shift+1")
        tab_chord(strata, "n")
        strata.open_directory(f"tab-{number}")
        selected_tab(strata, f"tab-{number}")
    tab_chord(strata, "1")
    selected_tab(strata, root)
    tab_chord(strata, "0")
    selected_tab(strata, "tab-10")
    strata.keyboard.press("ctrl+shift+1")
    selected_tab(strata, root)
    strata.keyboard.press("ctrl+shift+0")
    selected_tab(strata, "tab-10")
    tab_chord(strata, "x")
    selected_tab(strata, "tab-9")
    tab_chord(strata, "0")
    selected_tab(strata, "tab-9")


def test_ctrl_drop_on_a_tab_copies_without_removing_the_source(strata):
    root = strata.fixture.root.name
    strata.keyboard.press("ctrl+t")
    strata.open_directory("archive")
    selected_tab(strata, "archive")
    strata.keyboard.press("ctrl+shift+1")
    selected_tab(strata, root)
    source = strata.select_entry("todo.txt")
    strata.pointer.drag_points(
        strata.pointer.drag_origin(source),
        tab(strata, "archive").screen_bounds().center,
        modifiers=["ctrl"],
    )
    strata.wait(lambda: strata.fixture.path("archive/todo.txt").exists(), "file copied across tabs")
    assert strata.fixture.path("todo.txt").read_bytes() == strata.fixture.path("archive/todo.txt").read_bytes()
