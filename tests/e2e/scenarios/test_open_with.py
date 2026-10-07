# SPDX-License-Identifier: MIT

from configparser import ConfigParser

import pytest
from gi.repository import Gio


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

@pytest.fixture
def activation_fallback_app(open_with_app):
    output, associations, _ = open_with_app
    contents = associations.read_text().replace(
        "text/plain=strata-review.desktop;\n",
        "",
    )
    associations.write_text(contents)
    return output, associations, contents


@pytest.fixture
def empty_application_data(test_environment, monkeypatch):
    data_dirs = test_environment.root / "empty-data-dirs"
    data_dirs.mkdir()
    variables = test_environment.variables
    monkeypatch.setattr(
        test_environment,
        "variables",
        lambda: {**variables(), "XDG_DATA_DIRS": str(data_dirs)},
    )


@pytest.mark.parametrize("always_use", [False, True])
def test_activation_without_default_opens_with_visible_application(
    activation_fallback_app, strata, always_use
):
    output, associations, contents = activation_fallback_app
    expected = strata.fixture.path("todo.txt")
    strata.select_entry_with_keyboard("todo.txt")
    strata.keyboard.press("Return")

    dialog = strata.wait_for_dialog()
    assert "Review Text Viewer" in dialog.dump()
    strata.keyboard.type_text("Review Text Viewer")
    if always_use:
        toggle = dialog.find(role="check box", name="Always use for this file type")
        strata.pointer.click(toggle)
        strata.wait(lambda: "checked" in toggle.states, "the default toggle to check")
        strata.pointer.click(strata.dialog_button("Open"))
    else:
        strata.keyboard.press("Return")
    strata.wait(
        lambda: output.exists() and output.read_text(),
        "the selected application to receive the activated file",
    )

    received = output.read_text().splitlines()
    assert len(received) == 1
    assert Gio.File.new_for_commandline_arg(received[0]).equal(
        Gio.File.new_for_path(str(expected))
    )
    if always_use:
        defaults = ConfigParser()
        defaults.read(associations)
        content_type = Gio.File.new_for_path(str(expected)).query_info(
            "standard::content-type", Gio.FileQueryInfoFlags.NONE, None
        ).get_content_type()
        assert defaults["Default Applications"][content_type].split(";")[0] == "strata-review.desktop"
    else:
        assert associations.read_text() == contents
    strata.wait(lambda: strata.dialog() is None, "the chooser to close")
    strata.wait_for_focused_entry("todo.txt")
    if always_use:
        output.unlink()
        strata.keyboard.press("Return")
        strata.wait(lambda: output.exists() and output.read_text(), "the new default to open directly")
        assert strata.dialog() is None


def test_activation_without_selectable_application_shows_specific_empty_state(
    empty_application_data, strata
):
    strata.select_entry_with_keyboard("todo.txt")
    strata.keyboard.press("Return")

    dialog = strata.wait_for_dialog()
    strata.wait(
        lambda: dialog.find(
            role="label", name="No application is registered for this file"
        )
        is not None,
        "the activation-specific empty feedback",
        timeout=3.0,
    )
    assert "sensitive" not in strata.dialog_button("Open").states

    # The hidden "Always use" toggle must stay out of the focus cycle.
    for chord in ["shift+Tab", "Tab"]:
        strata.keyboard.press(chord)
        strata.wait(
            lambda: strata.focused_node() is not None
            and "visible" in strata.focused_node().states,
            f"{chord} to keep focus on a visible control",
        )

    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "the empty chooser to close")
    strata.wait_for_focused_entry("todo.txt")


@pytest.mark.parametrize("target", ["todo.txt", "documents", "background"])
def test_open_with_launches_without_changing_default(open_with_app, strata, target):
    output, associations, contents = open_with_app
    if target == "background":
        expected = strata.fixture.root
        strata.pointer.right_click(strata.pane(), at=strata.background_point())
        strata.wait(lambda: "Open With…" in strata.menu_items(), "folder menu")
    else:
        expected = strata.fixture.path(target)
        strata.open_context_menu(target)
        strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "MIME lookup")
    strata.choose_menu_item("Open With…")
    dialog = strata.wait_for_dialog()
    assert "Review Text Viewer" in dialog.dump()
    strata.keyboard.press("Return")
    strata.wait(
        lambda: output.exists() and output.read_text(),
        "the selected application to receive the target",
    )
    received = output.read_text().splitlines()
    assert len(received) == 1
    assert Gio.File.new_for_commandline_arg(received[0]).equal(
        Gio.File.new_for_path(str(expected))
    )
    assert associations.read_text() == contents
    strata.wait(lambda: strata.dialog() is None, "the chooser to close")


@pytest.mark.parametrize("target", ["todo.txt", "mixed", "documents", "background"])
def test_open_with_always_use_updates_the_default(chooser_apps, strata, target):
    output, associations, _ = chooser_apps
    if target == "background":
        expected = [strata.fixture.root]
        strata.pointer.right_click(strata.pane(), at=strata.background_point())
        strata.wait(lambda: "Open With…" in strata.menu_items(), "folder menu")
    else:
        names = ["todo.txt", "readme.md"] if target == "mixed" else [target]
        expected = [strata.fixture.path(name) for name in names]
        if target == "mixed":
            strata.select_entry("todo.txt")
            strata.pointer.click(strata.entry("readme.md"), modifiers=["ctrl"])
            strata.wait_for_selection(sorted(names))
        strata.open_context_menu(names[0])
        strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "MIME lookup")
    strata.choose_menu_item("Open With…")
    dialog = strata.wait_for_dialog()
    strata.keyboard.type_text("Alternative")
    strata.keyboard.press("Tab")
    strata.wait(lambda: strata.focused_node().name == "Alternative Viewer", "row focus")
    strata.keyboard.press("Tab")
    label = (
        "Always use for these file types"
        if target == "mixed"
        else "Always use for this file type"
    )
    strata.wait(
        lambda: strata.focused_node().name == label,
        "default toggle focus",
    )
    strata.keyboard.press("space")
    strata.wait(
        lambda: "checked" in dialog.find(role="check box", name=label).states,
        "the default toggle to check",
    )
    strata.keyboard.press("Tab")
    strata.keyboard.press("Tab")
    strata.wait(lambda: strata.focused_node().name == "Open", "Open focus")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: output.exists() and output.read_text(),
        "the selected application to receive the file",
    )
    received = [Gio.File.new_for_commandline_arg(value) for value in output.read_text().splitlines()]
    assert len(received) == len(expected)
    defaults = ConfigParser()
    defaults.read(associations)
    for path in expected:
        file = Gio.File.new_for_path(str(path))
        assert any(file.equal(opened) for opened in received)
        content_type = file.query_info(
            "standard::content-type", Gio.FileQueryInfoFlags.NONE, None
        ).get_content_type()
        assert defaults["Default Applications"][content_type].split(";")[0] == "strata-alternative.desktop"
    strata.wait(lambda: strata.dialog() is None, "the chooser to close")


def test_open_with_cancel_does_not_save_a_checked_default(chooser_apps, strata):
    output, associations, contents = chooser_apps
    for attempt in range(2):
        strata.open_context_menu("todo.txt")
        strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "MIME lookup")
        strata.choose_menu_item("Open With…")
        dialog = strata.wait_for_dialog()
        toggle = dialog.find(role="check box", name="Always use for this file type")
        assert "checked" not in toggle.states
        if attempt == 0:
            strata.pointer.click(toggle)
            strata.wait(lambda: "checked" in toggle.states, "the default toggle to check")
        strata.pointer.click(strata.dialog_button("Cancel"))
        strata.wait(lambda: strata.dialog() is None, "the chooser to close")
        assert associations.read_text() == contents
        assert not output.exists()


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
    strata.wait(
        lambda: strata.focused_node() is not None and "editable" in strata.focused_node().states,
        "search entry focused on open",
    )
    strata.keyboard.press("Down")
    strata.wait(lambda: "editable" in strata.focused_node().states, "search retains focus")
    strata.wait(
        lambda: any(row.name == "Alternative Viewer" and "selected" in row.states
                    for row in strata.dialog().find_all(role="list item")),
        "arrow selection",
    )
    assert "editable" in strata.focused_node().states
    strata.keyboard.press("Tab")
    strata.wait(lambda: strata.focused_node().name == "Alternative Viewer", "Tab into list")
    strata.keyboard.press("Tab")
    strata.wait(
        lambda: strata.focused_node().name == "Always use for this file type",
        "Tab to the default toggle",
    )
    strata.keyboard.press("Tab")
    strata.wait(lambda: strata.focused_node().name == "Cancel", "Tab to leave the toggle")
    strata.keyboard.press("shift+Tab")
    strata.wait(
        lambda: strata.focused_node().name == "Always use for this file type",
        "Shift+Tab returns to the toggle",
    )
    strata.keyboard.press("shift+Tab")
    strata.wait(lambda: strata.focused_node().name == "Alternative Viewer", "selected row focus")
    strata.keyboard.press("Tab")
    strata.keyboard.press("Tab")
    strata.keyboard.press("Tab")
    strata.wait(lambda: strata.focused_node().name == "Open", "Tab to reach Open")


def test_open_with_search_filters_and_escape_clears(chooser_apps, strata, request):
    from harness.artifacts import ArtifactCollector

    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "sensitive" in strata.menu_item("Open With…").states, "MIME lookup")
    strata.choose_menu_item("Open With…")
    strata.wait_for_dialog()
    strata.keyboard.type_text("ALTERNATIVE")
    strata.keyboard.press("Down")
    strata.wait(lambda: "editable" in strata.focused_node().states, "search retains focus after Down")
    strata.keyboard.press("Up")
    assert "editable" in strata.focused_node().states
    strata.keyboard.type_text("x")
    strata.wait(
        lambda: "No matching applications were found." in strata.dialog().dump(),
        "typing after arrow navigation appends at the caret",
    )
    strata.keyboard.press("BackSpace")
    strata.wait(lambda: "Alternative Viewer" in strata.dialog().dump(), "Backspace restores match")
    collector = ArtifactCollector(test_name=request.node.name)
    strata.screenshot(collector.directory / "filtered-chooser.png")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("no-such-application-821")
    strata.wait(
        lambda: "No matching applications were found." in strata.dialog().dump(),
        "empty search feedback",
    )
    strata.keyboard.press("Return")
    assert strata.dialog() is not None
    strata.keyboard.press("Escape")
    strata.wait(lambda: "No matching applications were found." not in strata.dialog().dump(), "cleared search")
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "dismissed chooser")


@pytest.mark.parametrize("action", ["Open", "Open With…"])
def test_open_with_mixed_types_share_a_hidden_default(chooser_apps, strata, action):
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
    assert "Image Viewer" in dump
    assert "Review Text Viewer" in dump
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "the chooser to close")


@pytest.mark.preferences(
    tenxer_mode=True,
    type_to_search=False,
    single_click_previews=False,
)
def test_tenxer_open_with_uses_the_fill_and_launches_only_on_accept(open_with_app, strata):
    output, associations, contents = open_with_app
    strata.select_entry_with_keyboard("documents")
    strata.keyboard.press("space")
    strata.select_entry_with_keyboard("todo.txt")
    strata.keyboard.press("space")
    strata.select_entry_with_keyboard("readme.md")

    strata.keyboard.press("O")
    dialog = strata.wait_for_dialog()
    assert "Review Text Viewer" in dialog.dump()
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "Esc to cancel the chooser")
    assert not output.exists(), "cancelling launches nothing"

    strata.keyboard.press("O")
    strata.wait_for_dialog()
    strata.keyboard.press("Return")
    strata.wait(
        lambda: output.exists() and len(output.read_text().splitlines()) == 2,
        "the application to receive the whole fill",
    )
    received = {Gio.File.new_for_commandline_arg(line).get_path() for line in output.read_text().splitlines()}
    assert received == {
        str(strata.fixture.path("documents")),
        str(strata.fixture.path("todo.txt")),
    }
    assert associations.read_text() == contents
    strata.wait(lambda: strata.dialog() is None, "the chooser to close")
