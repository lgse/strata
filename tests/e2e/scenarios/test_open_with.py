# SPDX-License-Identifier: MIT

import pytest
from gi.repository import Gio

from harness.modes import ALL_MODES


@pytest.fixture
def open_with_app(test_environment):
    applications = test_environment.data_home / "applications"
    applications.mkdir()
    output = test_environment.root / "opened-files"
    launcher = test_environment.root / "record-files"
    launcher.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" > "{output}"\n')
    launcher.chmod(0o755)
    (applications / "strata-review.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Review Text Viewer\n"
        f"Exec={launcher} %U\nMimeType=text/plain;inode/directory;\nNoDisplay=false\n"
    )
    associations = test_environment.config_home / "mimeapps.list"
    contents = (
        "[Default Applications]\ntext/plain=strata-review.desktop;\n"
        "inode/directory=strata-review.desktop;\n"
        "[Added Associations]\ntext/plain=strata-review.desktop;\n"
        "inode/directory=strata-review.desktop;\n"
    )
    associations.write_text(contents)
    return output, associations, contents


@pytest.mark.parametrize("mode", ALL_MODES)
def test_open_with_launches_selected_file_without_changing_default(
    open_with_app, strata, mode
):
    output, associations, contents = open_with_app
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "MIME lookup")
    strata.choose_menu_item("Open With…")
    dialog = strata.wait_for_dialog()
    assert "Review Text Viewer" in dialog.dump()
    strata.keyboard.press("Return")
    strata.wait(
        lambda: output.exists() and output.read_text(),
        "the selected application to receive the file",
    )
    received = output.read_text().splitlines()
    assert len(received) == 1
    assert Gio.File.new_for_commandline_arg(received[0]).equal(
        Gio.File.new_for_path(str(strata.fixture.path("todo.txt")))
    )
    assert associations.read_text() == contents
    strata.wait(lambda: strata.dialog() is None, "the chooser to close")


def test_open_with_launch_failure_shows_an_error(open_with_app, strata):
    output, associations, contents = open_with_app
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "MIME lookup")
    strata.choose_menu_item("Open With…")
    strata.wait_for_dialog()
    (output.parent / "record-files").unlink()
    strata.keyboard.press("Return")
    strata.wait(
        lambda: strata.dialog() is not None
        and strata.dialog().name == "Unable to open file",
        "the launch error dialog",
    )
    assert not output.exists()
    assert associations.read_text() == contents
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "the error dialog to close")


@pytest.fixture
def chooser_apps(open_with_app, test_environment):
    output, associations, _ = open_with_app
    applications = test_environment.data_home / "applications"
    launcher = test_environment.root / "record-files"
    for filename, name, extra in [
        ("strata-review", "Review Text Viewer", "NoDisplay=true\n"),
        ("strata-alternative", "Alternative Viewer", "Icon=strata-nonexistent-icon-569\n"),
        ("strata-missing", "Missing Icon Viewer", ""),
        ("strata-other-desktop", "Other Desktop Viewer", "OnlyShowIn=StrataTestDesktop;\n"),
    ]:
        (applications / f"{filename}.desktop").write_text(
            f"[Desktop Entry]\nType=Application\nName={name}\n"
            f"Exec={launcher} %U\nMimeType=text/plain;text/markdown;\n{extra}"
        )
    ids = "strata-review.desktop;strata-alternative.desktop;strata-missing.desktop;strata-other-desktop.desktop;"
    contents = (
        "[Default Applications]\n"
        "text/plain=strata-review.desktop;\ntext/markdown=strata-review.desktop;\n"
        f"[Added Associations]\ntext/plain={ids}\ntext/markdown={ids}\n"
    )
    associations.write_text(contents)
    return output, associations, contents


def test_open_with_names_rows_and_tabs_out_of_the_list(chooser_apps, strata):
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "MIME lookup")
    strata.choose_menu_item("Open With…")
    dialog = strata.wait_for_dialog()
    # Section headers are not selectable and carry their text in a child label.
    all_rows = dialog.find_all(role="list item")
    sections: dict[str, list[str]] = {}
    current_section = None
    for row in all_rows:
        if "selectable" not in row.states:
            current_section = next((c.name for c in row.children if c.name), "")
            sections.setdefault(current_section, [])
        elif current_section is not None:
            sections[current_section].append(row.name)
    recommended = sections.get("Recommended Applications", [])
    assert recommended[0] == "Review Text Viewer"
    assert {"Alternative Viewer", "Missing Icon Viewer"} <= set(recommended)
    assert recommended[1:] == sorted(recommended[1:], key=str.lower)
    assert all(recommended)
    assert "Other Desktop Viewer" not in [row.name for row in all_rows]
    # The search entry is focused on open; Down enters the list at the selected row.
    strata.wait(
        lambda: strata.focused_node() is not None and "editable" in strata.focused_node().states,
        "search entry focused on open",
    )
    strata.keyboard.press("Down")
    strata.wait(lambda: strata.focused_node().name == "Review Text Viewer", "initial row focus")
    strata.keyboard.press("Down")
    strata.wait(lambda: strata.focused_node().name == "Alternative Viewer", "arrow navigation")
    strata.keyboard.press("Tab")
    strata.wait(lambda: strata.focused_node().name == "Cancel", "Tab to leave the list")
    strata.keyboard.press("shift+Tab")
    strata.wait(lambda: strata.focused_node().name == "Alternative Viewer", "selected row focus")
    strata.keyboard.press("Tab")
    strata.keyboard.press("Tab")
    strata.wait(lambda: strata.focused_node().name == "Open", "Tab to reach Open")


@pytest.mark.parametrize("action", ["Open", "Open With…"])
@pytest.mark.parametrize("mode", ALL_MODES)
def test_open_with_mixed_types_share_a_hidden_default(chooser_apps, strata, action, mode):
    output, associations, contents = chooser_apps
    strata.select_entry("todo.txt")
    strata.pointer.click(strata.entry("readme.md"), modifiers=["ctrl"])
    strata.wait_for_selection(["readme.md", "todo.txt"])
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Open" in strata.menu_items(), "shared default lookup")
    strata.choose_menu_item(action)
    if action == "Open With…":
        strata.wait_for_dialog()
        strata.keyboard.press("Return")
    strata.wait(lambda: output.exists() and len(output.read_text().splitlines()) == 2, "both files to open")
    received = [Gio.File.new_for_commandline_arg(value) for value in output.read_text().splitlines()]
    for name in ["todo.txt", "readme.md"]:
        assert any(file.equal(Gio.File.new_for_path(str(strata.fixture.path(name)))) for file in received)
    assert associations.read_text() == contents


@pytest.fixture
def different_defaults(chooser_apps):
    _, associations, contents = chooser_apps
    associations.write_text(contents.replace(
        "text/markdown=strata-review.desktop;\n",
        "text/markdown=strata-alternative.desktop;\n",
        1,
    ))


def test_open_with_common_handlers_do_not_imply_a_shared_default(different_defaults, strata):
    strata.select_entry("todo.txt")
    strata.pointer.click(strata.entry("readme.md"), modifiers=["ctrl"])
    strata.wait_for_selection(["readme.md", "todo.txt"])
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "common handlers")
    assert "Open" not in strata.menu_items()
    strata.choose_menu_item("Open With…")
    assert "Alternative Viewer" in strata.wait_for_dialog().dump()


@pytest.fixture
def incompatible_files(fixture_tree, open_with_app, test_environment):
    fixture_tree.path("unknown.bin").write_bytes(bytes(range(256)))
    fixture_tree.path("broken-link").symlink_to("missing-target")
    fixture_tree.path("image.png").write_bytes(b"\x89PNG\r\n\x1a\n")
    associations = test_environment.config_home / "mimeapps.list"
    with associations.open("a") as stream:
        stream.write("image/png=strata-image.desktop;\n")
    applications = test_environment.data_home / "applications"
    (applications / "strata-image.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Image Viewer\nExec=/bin/true %U\nMimeType=image/png;\n"
    )


def test_open_with_broken_link_is_disabled(incompatible_files, strata):
    strata.open_context_menu("broken-link")
    strata.wait(
        lambda: "Broken symbolic links cannot be opened with an application"
        in strata.menu_item("Open With…").description,
        "MIME lookup result",
    )
    option = strata.menu_item("Open With…")
    assert "sensitive" not in option.states
    strata.keyboard.press("Escape")
    assert strata.dialog() is None


def test_open_with_unknown_type_offers_other_apps(incompatible_files, strata):
    strata.open_context_menu("unknown.bin")
    strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "other apps available")
    strata.choose_menu_item("Open With…")
    dialog = strata.wait_for_dialog()
    dump = dialog.dump()
    assert "Other Applications" in dump
    assert "Recommended Applications" not in dump
    assert "Image Viewer" in dump
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "the chooser to close")


def test_open_with_incompatible_types_offers_other_apps(incompatible_files, strata):
    strata.select_entry("todo.txt")
    strata.pointer.click(strata.entry("image.png"), modifiers=["ctrl"])
    strata.wait_for_selection(["image.png", "todo.txt"])
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "other apps available")
    assert "Open" not in strata.menu_items()
    strata.choose_menu_item("Open With…")
    dialog = strata.wait_for_dialog()
    dump = dialog.dump()
    assert "Other Applications" in dump
    assert "Recommended Applications" not in dump
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "the chooser to close")


@pytest.mark.parametrize("mode", ALL_MODES)
def test_open_with_launches_selected_folder(open_with_app, strata, mode):
    output, associations, contents = open_with_app
    strata.open_context_menu("documents")
    strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "MIME lookup")
    strata.choose_menu_item("Open With…")
    assert "Review Text Viewer" in strata.wait_for_dialog().dump()
    strata.keyboard.press("Return")
    strata.wait(
        lambda: output.exists() and output.read_text(),
        "the selected application to receive the folder",
    )
    received = output.read_text().splitlines()
    assert len(received) == 1
    assert Gio.File.new_for_commandline_arg(received[0]).equal(
        Gio.File.new_for_path(str(strata.fixture.path("documents")))
    )
    assert associations.read_text() == contents
