# SPDX-License-Identifier: MIT
"""Column background clicks focus the parent without closing its descendants."""

import pytest


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
@pytest.mark.parametrize("surface", ["item", "background"])
@pytest.mark.parametrize("previous", ["root", "nested"])
def test_context_menu_keeps_the_clicked_column_target(strata, surface, previous):
    root = strata.fixture.root.name
    nested = strata.fixture.path("documents/nested")
    nested.mkdir()
    (nested / "child.txt").write_text("child")
    strata.open_directory("documents")
    strata.open_directory("nested", directory="documents")
    previous_directory = root if previous == "root" else "nested"
    strata.select_entry(
        "readme.md" if previous == "root" else "child.txt",
        directory=previous_directory,
    )
    if surface == "item":
        strata.open_context_menu("notes.txt", directory="documents")
    else:
        strata.pointer.click(
            strata.pane("documents"),
            at=strata.background_point("documents"),
            button=3,
        )
        strata.wait(strata.context_menu, "the background context menu to open")

    def action_owners():
        return [name for name in strata.pane_names()
                if strata.pane(name).find(role="button", name="Refresh (F5)") is not None]

    strata.wait(lambda: action_owners() == ["documents"], "actions to stay on the menu's column")
    strata.pointer.move_to(*strata.pane(previous_directory).screen_bounds().center)
    assert strata.context_menu() is not None
    assert action_owners() == ["documents"]
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.context_menu() is None, "the context menu to close")
    strata.keyboard.press("Down")
    strata.wait_for_focused_entry("report.md" if surface == "item" else "notes.txt")
    strata.wait(lambda: action_owners() == ["documents"], "keyboard navigation to resume in the menu's column")


@pytest.mark.preferences(browser_mode="columns")
@pytest.mark.parametrize("surface", ["content", "header"])
def test_column_background_click_focuses_parent(strata, surface):
    root = strata.fixture.root.name
    strata.open_directory("documents")
    selected = strata.selected_names(directory=root)
    if surface == "content":
        strata.pointer.click(strata.pane(root), at=strata.background_point(root))
    else:
        heading = strata.window.find(
            role="label", name=root, description=str(strata.fixture.root)
        )
        assert heading is not None
        strata.pointer.click(heading)
    strata.wait_for_directory(root)
    if surface == "content":
        strata.wait_for_selection(["archive"], root)
        strata.pointer.click(strata.pane(root), at=strata.background_point(root))
        strata.wait(lambda: not strata.all_selected_names(), "active background click to clear selection")
    else:
        assert strata.selected_names(directory=root) == selected
    assert "documents" in strata.pane_names()
