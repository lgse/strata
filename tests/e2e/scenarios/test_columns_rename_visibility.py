# SPDX-License-Identifier: GPL-3.0-or-later
"""Committed names must remain painted inside the Columns content viewport."""

import pytest


def wait_for_visible_commit(strata, name):
    def visible():
        pane = strata.pane("rename-target")
        entry = pane.find(role="list item", name=name)
        footer = pane.find(role="label", name_matches="Paste here")
        if entry is None or footer is None or not entry.has_state("selected"):
            return False
        panel = entry.find(role="panel")
        if panel is None:
            return False
        bounds = panel.screen_bounds()
        viewport = strata.entry_container("rename-target").screen_bounds()
        return (
            bounds.width > 0 and bounds.height > 0
            and bounds.x >= viewport.x and bounds.y >= viewport.y
            and bounds.x + bounds.width <= viewport.x + viewport.width
            and bounds.y + bounds.height <= min(
                viewport.y + viewport.height, footer.screen_bounds().y
            )
        )

    strata.wait(visible, f"{name} selected and fully above the footer")
    strata.settle(strata.entry(name, "rename-target").find(role="panel"))
    assert visible()


def begin_long_directory_rename(strata, kind, new):
    strata.switch_view("Columns")
    strata.fixture.path("rename-target").mkdir()
    for index in range(80):
        path = strata.fixture.path(f"rename-target/m-entry-{index:03d}")
        if kind == "folder":
            path.mkdir()
            (path / "marker").write_text("body\n")
        else:
            path.write_text("body\n")
    strata.keyboard.press("F5")
    strata.entry("rename-target")
    strata.open_directory("rename-target")
    strata.keyboard.press("Home")
    strata.keyboard.press("End")
    original = "m-entry-079"
    strata.wait_for_selection([original], "rename-target")
    if new:
        # Aim creation at the parent, not the selected folder's child column.
        bounds = strata.entry_container("rename-target").screen_bounds()
        strata.pointer.right_click(
            strata.pane("rename-target"),
            at=(bounds.x + 1, bounds.y + bounds.height // 2),
        )
        strata.choose_menu_item("New File" if kind == "file" else "New Folder")
        original = "new " + kind
        strata.wait(
            strata.fixture.path("rename-target/" + original).exists,
            "the new item on disk",
        )
    else:
        strata.keyboard.press("F2")
    field = strata.editable_field()
    strata.wait(lambda: field.text == original, "the original name in the editor")
    def editor_visible():
        viewport = strata.entry_container("rename-target").screen_bounds()
        footer = strata.pane("rename-target").find(role="label", name_matches="Paste here")
        bounds = field.screen_bounds()
        return (bounds.height > 0 and bounds.width > 0
                and bounds.y >= viewport.y
                and bounds.y + bounds.height <= footer.screen_bounds().y)

    strata.wait(editor_visible, "the initial editor above the footer without scrolling")
    strata.settle(field)
    assert editor_visible()
    path = strata.fixture.path("rename-target/" + original)
    assert path.is_dir() if kind == "folder" else path.is_file()
    return field, original


@pytest.mark.parametrize("kind", ("file", "folder"))
@pytest.mark.parametrize("new", (False, True), ids=("existing", "new"))
@pytest.mark.parametrize("final_name", ("a-final", "m-entry-078a", "zz-final"))
def test_columns_committed_rename_visibility(strata, kind, new, final_name):
    field, original = begin_long_directory_rename(strata, kind, new)
    strata.keyboard.type_text(final_name)
    strata.wait(lambda: field.text == final_name, "the committed name in the editor")
    strata.keyboard.press("Return")
    destination = strata.fixture.path("rename-target/" + final_name)
    strata.wait(destination.exists, "the renamed item on disk")
    strata.wait_for_entry_gone(original, "rename-target")
    if kind == "file":
        assert destination.read_text() == ("" if new else "body\n")
    else:
        assert destination.is_dir()
        if not new:
            assert (destination / "marker").read_text() == "body\n"
    assert not strata.fixture.path("rename-target/" + original).exists()
    wait_for_visible_commit(strata, final_name)
    names = sorted(path.name for path in strata.fixture.path("rename-target").iterdir())
    position = names.index(final_name)
    key, neighbor = ("Down", names[position + 1]) if position == 0 else ("Up", names[position - 1])
    strata.keyboard.press(key)
    strata.wait_for_selection([neighbor], "rename-target")
    strata.keyboard.press("Up" if key == "Down" else "Down")
    strata.wait_for_selection([final_name], "rename-target")
    wait_for_visible_commit(strata, final_name)


@pytest.mark.parametrize("kind", ("file", "folder"))
@pytest.mark.parametrize("new", (False, True), ids=("existing", "new"))
def test_columns_click_away_rename_respects_navigation(strata, kind, new):
    field, original = begin_long_directory_rename(strata, kind, new)
    strata.keyboard.type_text("a-final")
    strata.wait(lambda: field.text == "a-final", "the final name in the editor")
    # The root column remains visible while the long child column is scrolled.
    strata.pointer.click(strata.entry("documents", strata.fixture.root.name))
    destination = strata.fixture.path("rename-target/a-final")
    strata.wait(destination.exists, "the rename on disk")
    strata.wait_for_directory("documents")
    strata.wait_for_selection(["documents"], strata.fixture.root.name)
    assert not strata.fixture.path("rename-target/" + original).exists()
    assert "rename-target" not in strata.pane_names()
