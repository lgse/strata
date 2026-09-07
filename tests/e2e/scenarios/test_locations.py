# SPDX-License-Identifier: GPL-3.0-or-later
"""Opening locations directly and reacting to filesystem changes."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
def test_the_command_line_location_is_the_one_shown(strata, mode):
    assert strata.current_directory() == strata.fixture.root.name
    assert strata.entry_names() == [
        "archive",
        "documents",
        "pictures",
        "readme.md",
        "todo.txt",
    ]


def test_typing_a_path_navigates_there(strata):
    strata.keyboard.press("ctrl+l")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(strata.fixture.path("documents")))
    strata.wait(
        lambda: field.text.endswith("documents"),
        "the path to be typed into the address bar",
    )
    strata.keyboard.press("Return")

    strata.wait_for_directory("documents")
    strata.entry("notes.txt", directory="documents")


def test_a_breadcrumb_returns_to_the_parent(strata):
    strata.open_directory("documents")

    crumb = strata.wait(
        lambda: strata.window.find(role="button", name=strata.fixture.root.name),
        "the breadcrumb for the fixture root",
    )
    strata.pointer.click(crumb)

    strata.wait_for_directory(strata.fixture.root.name)


def test_a_sidebar_place_navigates_there(strata):
    home = strata.environment.home
    (home / "sidebar-target.txt").write_text("target\n")

    strata.pointer.click(strata.sidebar_button("Home"))

    strata.wait_for_directory(home.name)
    strata.entry("sidebar-target.txt")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_a_file_created_outside_appears_after_a_refresh(strata, mode):
    strata.fixture.path("appeared-later.txt").write_text("new\n")

    strata.keyboard.press("F5")

    strata.entry("appeared-later.txt")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_a_file_removed_outside_disappears_after_a_refresh(strata, mode):
    strata.entry("todo.txt")
    strata.fixture.path("todo.txt").unlink()

    strata.keyboard.press("F5")

    strata.wait_for_entry_gone("todo.txt")


def test_an_unreadable_location_reports_an_error(strata):
    blocked = strata.fixture.path("blocked")
    blocked.mkdir()
    blocked.chmod(0o000)
    try:
        strata.keyboard.press("F5")
        strata.click_entry("blocked")

        dialog = strata.wait_for_dialog()
        assert dialog.name == "Unable to open directory", (
            f"unexpected dialog {dialog.name!r}"
        )
        assert any(
            "do not have permission" in node.name
            for node in dialog.find_all(role="label")
        ), f"the dialog should give the reason\n{dialog.dump()}"

        strata.pointer.click(strata.dialog_button("Close"))
        strata.wait(lambda: strata.dialog() is None, "the dialog to close")
    finally:
        blocked.chmod(0o755)
