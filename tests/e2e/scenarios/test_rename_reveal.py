# SPDX-License-Identifier: GPL-3.0-or-later
"""Real-input regression coverage for revealing a renamed entry."""

import os
from pathlib import Path

import pytest

from harness.modes import ALL_MODES


def fully_above_footer(node, footer) -> bool:
    bounds = node.screen_bounds()
    footer_bounds = footer.screen_bounds()
    return bounds.height > 0 and bounds.y + bounds.height <= footer_bounds.y


def destination_hint(strata, directory: str | None = None):
    def current_hint():
        pane = strata._pane_or_none(directory)
        if pane is None:
            return None
        return pane.find(role="label", name_matches="Paste here")

    return strata.wait(current_hint, "the Columns destination footer")


def wait_for_stable_selected_entry(strata, name: str, directory: str | None = None):
    stable = {"identity": None, "count": 0}

    def check():
        pane = strata._pane_or_none(directory)
        if pane is None:
            stable["count"] = 0
            return False
        entry = pane.find(role="list item", name=name)
        footer = pane.find(role="label", name_matches="Paste here")
        if entry is None or footer is None:
            stable["count"] = 0
            return False
        panel = entry.find(role="panel")
        label = entry.find(role="label", name=name)
        if panel is None or label is None:
            stable["count"] = 0
            return False
        bounds = panel.screen_bounds()
        label_bounds = label.screen_bounds()
        footer_bounds = footer.screen_bounds()
        pane_bounds = pane.screen_bounds()
        geometry = (bounds, label_bounds, footer_bounds)
        visible = (
            fully_in_pane(panel, pane_bounds)
            and fully_above_footer(panel, footer)
            and fully_above_footer(label, footer)
            and entry.has_state("selected")
            and entry.has_state("focused")
        )
        if visible and geometry == stable["identity"]:
            stable["count"] += 1
        else:
            stable["identity"] = geometry
            stable["count"] = 0
        return stable["count"] >= 10

    strata.wait(check, f"{name} to remain stable above the footer")


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


@pytest.mark.parametrize("height", (300, 420))
def test_columns_last_sorted_entry_stays_painted_above_footer_after_rename(strata, height):
    strata.switch_view("Columns")
    bounds = strata.window.window_bounds()
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, 420, height)
    strata.wait(lambda: strata.window.window_bounds().height == height, f"a {height}px window")
    for index in range(240):
        strata.fixture.path(f"m-file-{index:04d}.txt").write_text("body\\n")
        strata.fixture.path(f"m-folder-{index:04d}").mkdir()

    strata.keyboard.press("F5")
    strata.wait(strata.fixture.path("m-file-0000.txt").exists, "the long listing")
    strata.entry("archive")
    strata.keyboard.press("Home")
    strata.pointer.right_click(strata.pane(), at=strata.folder_context_point())
    strata.choose_menu_item("New File")
    field = strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable", "focused"}),
        "the focused new-entry rename editor",
    )
    strata.wait(lambda: field.text == "new file", "the default new-file name")
    strata.keyboard.type_text("zzzzzz")
    strata.wait(lambda: field.text == "zzzzzz", "the final name in the editor")
    strata.keyboard.press("Return")
    strata.wait(strata.fixture.path("zzzzzz").exists, "the renamed final entry on disk")
    strata.wait_for_entry_gone("new file")
    strata.wait_for_selection(["zzzzzz"])
    assert strata.fixture.names()[-1] == "zzzzzz"

    footer = destination_hint(strata)
    stable = {"last": None, "count": 0}

    def painted_and_stable():
        entry = strata.entry("zzzzzz")
        panel = entry.find(role="panel")
        name_bounds = entry.find(role="label", name="zzzzzz").screen_bounds()
        entry_bounds = panel.screen_bounds()
        footer_bounds = footer.screen_bounds()
        current = (
            entry_bounds.x,
            entry_bounds.y,
            entry_bounds.width,
            entry_bounds.height,
            name_bounds.x,
            name_bounds.y,
            name_bounds.width,
            name_bounds.height,
        )
        fully_painted = (
            entry_bounds.height > 0
            and entry_bounds.y + entry_bounds.height <= footer_bounds.y
            and name_bounds.height > 0
            and name_bounds.y + name_bounds.height <= footer_bounds.y
        )
        if current == stable["last"] and fully_painted and strata.selected_names() == ["zzzzzz"]:
            stable["count"] += 1
        else:
            stable["last"] = current
            stable["count"] = 0
        return stable["count"] >= 10

    strata.wait(painted_and_stable, f"zzzzzz to remain painted above the footer at {height}px")
    assert strata.selected_names() == ["zzzzzz"]


@pytest.mark.parametrize("kind", ("file", "folder"))
def test_columns_new_entry_stays_above_footer(strata, kind):
    strata.switch_view("Columns")
    for index in range(160):
        strata.fixture.path(f"b-entry-{index:03d}").mkdir(exist_ok=True)

    strata.keyboard.press("F5")
    strata.entry("b-entry-000")
    strata.keyboard.press("Home")
    if kind == "file":
        strata.pointer.right_click(strata.pane(), at=strata.folder_context_point())
        strata.choose_menu_item("New File")
    else:
        strata.keyboard.press("ctrl+shift+n")
    field = strata.editable_field()
    strata.wait(lambda: field.text == "new " + kind, f"the default new-{kind} name")
    strata.wait(strata.fixture.path("new " + kind).exists, f"the new {kind} on disk")
    footer = destination_hint(strata)
    strata.wait(
        lambda: fully_above_footer(field, footer),
        f"the new {kind} rename editor to stay above the Columns footer",
    )
    capture_evidence(strata, "before")
    final_name = "zz-created-" + kind
    strata.keyboard.type_text(final_name)
    strata.wait(lambda: field.text == final_name, "the final name in the editor")
    strata.keyboard.press("Return")
    strata.wait(strata.fixture.path(final_name).exists, "the renamed item on disk")
    strata.wait_for_entry_gone("new " + kind)
    strata.wait_for_selection([final_name])
    # GTK 4.14 exports extra outer-row padding in ListItem accessibility bounds.
    # Check the painted file-row content, matching the native GTK fixture.
    strata.wait(
        lambda: fully_above_footer(
            strata.entry(final_name).find(role="panel"), destination_hint(strata)
        ),
        "the final selected item content to stay above the Columns footer",
    )
    capture_evidence(strata, "after")


@pytest.mark.preferences(reduce_motion=False, theme="3024")
@pytest.mark.parametrize("final_name", ("m-file-0118a.txt", "zzzz"))
def test_columns_rename_focus_follows_the_item_instead_of_its_old_position(strata, final_name):
    strata.switch_view("Columns")
    for index in range(120):
        strata.fixture.path(f"m-file-{index:04d}.txt").write_text("body\n")
    for index in range(24):
        strata.fixture.path(f"z-tail-{index:04d}.txt").write_text("body\n")
    strata.keyboard.press("F5")
    strata.entry("m-file-0000.txt")
    strata.pointer.right_click(strata.pane(), at=strata.folder_context_point())
    strata.choose_menu_item("New File")
    field = strata.editable_field()
    strata.wait(lambda: field.text == "new file", "the default file name")
    strata.pointer.move_to(*field.screen_bounds().center)
    anchor = strata.entry("m-file-0118.txt").find(role="label", name="m-file-0118.txt")
    assert fully_in_pane(strata.settle(anchor), strata.pane().screen_bounds())

    strata.keyboard.type_text(final_name)
    strata.wait(lambda: field.text == final_name, "the replacement name")
    strata.keyboard.press("Return")
    strata.wait(strata.fixture.path(final_name).exists, "the renamed file on disk")
    strata.wait_for_entry_gone("new file")
    wait_for_stable_selected_entry(strata, final_name)
    if final_name == "m-file-0118a.txt":
        assert fully_in_pane(
            strata.entry("m-file-0118.txt").find(role="label", name="m-file-0118.txt"),
            strata.pane().screen_bounds(),
        )


@pytest.mark.preferences(reduce_motion=False)
def test_columns_realistic_tall_listing_collision_and_navigation(strata):
    """Exercise the reported path without manually scrolling after creation."""
    strata.switch_view("Columns")
    bounds = strata.window.window_bounds()
    strata.keyboard.connection.resize_surface(bounds.width, bounds.height, 1000, 850)
    strata.wait(lambda: strata.window.window_bounds().height == 850, "a tall window")

    strata.fixture.path("navigation/staging/target").mkdir(parents=True)
    target = strata.fixture.path("navigation/staging/target")
    for index in range(11):
        target.joinpath("new file" if index == 0 else f"new file ({index})").write_text("")
    for index in range(60):
        target.joinpath(f"b-entry-{index:03d}").write_text("")
    target.joinpath("y").write_text("")
    target.joinpath("yy").write_text("")
    target.joinpath("existing").write_text("")

    strata.keyboard.press("F5")
    strata.open_directory("navigation")
    strata.open_directory("staging")
    strata.open_directory("target")
    strata.wait(lambda: len(strata.fixture.names("navigation/staging/target")) == 74, "the realistic listing")

    strata.pointer.right_click(strata.pane(), at=strata.folder_context_point())
    strata.choose_menu_item("New File")
    field = strata.editable_field()
    strata.wait(lambda: field.text == "new file (11)", "the next collision-free default name")
    strata.pointer.move_to(*field.screen_bounds().center)
    strata.wait(target.joinpath("new file (11)").exists, "the created file")
    strata.keyboard.type_text("zzz")
    strata.wait(lambda: field.text == "zzz", "the replacement name")
    strata.keyboard.press("Return")
    strata.wait(target.joinpath("zzz").exists, "the renamed file")
    strata.wait_for_entry_gone("new file (11)")
    strata.wait_for_selection(["zzz"])

    wait_for_stable_selected_entry(strata, "zzz", "target")


def capture_evidence(strata, label: str) -> None:
    directory = os.environ.get("STRATA_RENAME_EVIDENCE_DIR")
    if directory:
        strata.screenshot(Path(directory) / f"{label}.png")
