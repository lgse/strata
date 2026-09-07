# SPDX-License-Identifier: GPL-3.0-or-later

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
        f"Exec={launcher} %U\nMimeType=text/plain;\nNoDisplay=false\n"
    )
    associations = test_environment.config_home / "mimeapps.list"
    contents = (
        "[Default Applications]\ntext/plain=strata-review.desktop;\n"
        "[Added Associations]\ntext/plain=strata-review.desktop;\n"
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


def test_open_with_is_hidden_for_directories(strata):
    strata.open_context_menu("documents")
    assert "Open With…" not in strata.menu_items()
    strata.dismiss_menu()
