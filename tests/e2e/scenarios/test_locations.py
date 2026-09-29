# SPDX-License-Identifier: MIT
"""Opening locations directly and reacting to filesystem changes."""

from __future__ import annotations

import pytest

from harness.modes import COLUMNS_AND_ONE


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


def test_named_location_controls_navigate_and_cancel(strata):
    strata.keyboard.press("ctrl+l")
    field = strata.window.find(role="text", name="Location (Ctrl+L)")
    assert field is not None, strata.window.dump()
    confirm = strata.window.find(role="button", name="Navigate (Enter)")
    cancel = strata.window.find(role="button", name="Cancel (Escape)")
    assert confirm is not None and cancel is not None, strata.window.dump()

    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(strata.fixture.path("documents")))
    strata.wait(lambda: field.text.endswith("documents"), "typed location")
    strata.pointer.click(confirm)
    strata.wait_for_directory("documents")
    strata.entry("notes.txt", directory="documents")

    strata.keyboard.press("ctrl+l")
    cancel = strata.window.find(role="button", name="Cancel (Escape)")
    assert cancel is not None
    strata.pointer.click(cancel)
    strata.wait(
        lambda: strata.window.find(role="button", name="Navigate (Enter)") is None,
        "the location entry to close",
    )
    assert strata.window.find(role="text", name="Location (Ctrl+L)") is None
    strata.wait_for_directory("documents")


def test_a_breadcrumb_returns_to_the_parent(strata):
    strata.open_directory("documents")

    crumb = strata.wait(
        lambda: strata.window.find(role="button", name=strata.fixture.root.name),
        "the breadcrumb for the fixture root",
    )
    strata.pointer.click(crumb)

    strata.wait_for_directory(strata.fixture.root.name)


def test_current_breadcrumb_opens_hierarchy_instead_of_window_menu(strata):
    path = strata.fixture.root
    for index in range(6):
        path = path / f"deep-breadcrumb-component-{index}"
    path.mkdir(parents=True)
    strata.entry("deep-breadcrumb-component-0")
    strata.keyboard.press("ctrl+l")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(path))
    strata.wait(lambda: field.text == str(path), "typed location")
    strata.keyboard.press("Return")
    strata.wait_for_directory(path.name)
    label = strata.wait(
        lambda: strata.window.find(role="label", name=path.name),
        "current breadcrumb",
    )
    strata.pointer.click(label, button=3)
    strata.wait(
        lambda: strata.window.find(role="button", name=path.name),
        "current hierarchy item",
    )
    item = strata.window.find_all(role="button", name=path.parent.name)[-1]
    strata.pointer.click(item)
    strata.wait_for_directory(path.parent.name)


def test_a_sidebar_place_navigates_there(strata):
    home = strata.environment.home
    (home / "sidebar-target.txt").write_text("target\n")

    strata.pointer.click(strata.sidebar_button("Home"))

    strata.wait_for_directory(home.name)
    strata.entry("sidebar-target.txt")


@pytest.fixture
def places(test_environment):
    """Downloads exists and Documents is missing; pins are stored beta,
    Downloads (hidden as a standard place), alpha."""

    home = test_environment.home
    for name in ("Downloads", "pins/beta", "pins/alpha"):
        (home / name).mkdir(parents=True)
    config = test_environment.config_home
    (config / "user-dirs.dirs").write_text(
        'XDG_DOWNLOAD_DIR="$HOME/Downloads"\nXDG_DOCUMENTS_DIR="$HOME/Documents"\n'
    )
    (config / "gtk-3.0").mkdir(exist_ok=True)
    (config / "gtk-3.0" / "bookmarks").write_text(
        "".join(
            f"{(home / name).as_uri()} {label}\n"
            for name, label in (
                ("pins/beta", "Beta"),
                ("Downloads", "Downloads"),
                ("pins/alpha", "Alpha"),
            )
        )
    )
    return home


@pytest.mark.preferences(tenxer_mode=True, type_to_search=False)
def test_tenxer_go_chord_jumps_to_places_and_visible_pins(places, strata):
    root = strata.current_directory()

    for second, message in (
        ("k", "No Documents folder"),
        ("3", "No pin 3"),
        ("z", "Unknown chord"),
    ):
        strata.keyboard.press("g")
        strata.keyboard.press(second)
        strata.wait(
            lambda: strata.window.find(role="label", name=message) is not None,
            f"g {second} to report {message!r}",
        )
        assert strata.current_directory() == root

    for second, directory in (
        ("h", places.name),
        ("d", "Downloads"),
        ("2", "alpha"),
        ("1", "beta"),
    ):
        strata.keyboard.press("g")
        strata.keyboard.press(second)
        strata.wait_for_directory(directory)

    strata.keyboard.press("g")
    strata.keyboard.press("Escape")
    strata.keyboard.press("h")
    strata.wait_for_directory("pins")


@pytest.mark.preferences(tenxer_mode=True, type_to_search=False)
def test_tenxer_go_prompt_completes_folders_and_navigates(strata):
    (strata.fixture.path("documents") / "drafts").mkdir()

    strata.keyboard.press("g")
    strata.keyboard.press("space")
    field = strata.editable_field()
    assert strata.window.find(role="text", name="Go to a path or URI") is not None
    for key, expected in (
        ("Tab", "archive/"),
        ("Tab", "documents/"),
        ("Tab", "pictures/"),
        ("shift+Tab", "documents/"),
    ):
        strata.keyboard.press(key)
        strata.wait(lambda: field.text == expected, f"{key} to complete {expected}")
    strata.keyboard.type_text("dr")
    strata.keyboard.press("Tab")
    strata.wait(
        lambda: field.text == "documents/drafts/",
        "Tab after a slash to complete from that folder",
    )
    strata.keyboard.press("Return")
    strata.wait_for_directory("drafts")

    secret = "sftp://user:hunter2@inert.invalid/srv/"
    strata.keyboard.press("g")
    strata.keyboard.press("space")
    field = strata.editable_field()
    strata.keyboard.type_text(secret)
    strata.keyboard.press("Tab")
    strata.wait(
        lambda: strata.window.find(role="label", name="URIs are not completed")
        is not None,
        "Tab to leave a URI alone",
    )
    assert field.text == secret
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="text", states={"editable", "focused"}) is None,
        "Escape to close the go prompt",
    )
    strata.wait_for_directory("drafts")

    strata.keyboard.press("g")
    strata.keyboard.press("space")
    field = strata.editable_field()
    assert field.text == "", "reopening recovers no typed text"
    strata.keyboard.press("Escape")


@pytest.mark.preferences(tenxer_mode=True, type_to_search=False)
def test_tenxer_history_prompts_jump_to_visited_folders(strata):
    for path, directory in (
        ("documents", "documents"),
        ("../pictures", "pictures"),
        ("../archive", "archive"),
    ):
        strata.keyboard.press("g")
        strata.keyboard.press("space")
        strata.editable_field()
        strata.keyboard.type_text(path)
        strata.keyboard.press("Return")
        strata.wait_for_directory(directory)

    strata.keyboard.press("z")
    field = strata.editable_field()
    assert strata.window.find(role="text", name="Jump to a visited folder") is not None
    strata.keyboard.type_text("qqqq")
    strata.wait(
        lambda: strata.window.find(role="label", name="No matching folders")
        is not None,
        "a miss to report no matching folders",
    )
    strata.keyboard.press("Return")
    assert strata.current_directory() == "archive", "a miss never navigates"
    for _ in "qqqq":
        strata.keyboard.press("BackSpace")
    strata.keyboard.type_text("doc")
    strata.wait(lambda: field.text == "doc", "the query to be typed")
    strata.keyboard.press("Return")
    strata.wait_for_directory("documents")

    strata.keyboard.press("shift+Z")
    assert (
        strata.window.find(role="text", name="Jump to a recently visited folder")
        is not None
    )
    strata.keyboard.press("Down")
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="text", states={"editable", "focused"}) is None,
        "Escape to close the recent prompt",
    )
    assert strata.current_directory() == "documents", "Escape opens nothing"

    strata.keyboard.press("shift+Z")
    strata.editable_field()
    strata.keyboard.type_text("arch")
    strata.keyboard.press("Return")
    strata.wait_for_directory("archive")


@pytest.mark.parametrize("mode", COLUMNS_AND_ONE)
def test_refresh_reconciles_external_file_creation_and_removal(strata, mode):
    strata.entry("todo.txt")
    strata.fixture.path("appeared-later.txt").write_text("new\n")
    strata.fixture.path("todo.txt").unlink()

    strata.keyboard.press("F5")

    strata.entry("appeared-later.txt")
    strata.wait_for_entry_gone("todo.txt")


def test_an_unreadable_location_reports_an_error(strata):
    blocked = strata.fixture.path("blocked")
    blocked.mkdir()
    blocked.chmod(0o000)
    try:
        strata.keyboard.press("F5")
        strata.select_entry("blocked")

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
