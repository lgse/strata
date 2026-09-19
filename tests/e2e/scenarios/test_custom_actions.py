# SPDX-License-Identifier: MIT
"""The action editor keeps drafts across tabs and writes the portable manifest."""

import hashlib
import tomllib


def _open_new_action(strata):
    assert strata.window.find(role="button", name="Settings").activate()
    actions = strata.wait(
        lambda: strata.window.find(role="button", name="Actions"), "Actions settings"
    )
    assert actions.activate()
    create = strata.wait(
        lambda: strata.window.find(role="button", name="New action…"), "New action"
    )
    assert create.activate()
    return strata.wait(
        lambda: strata.window.find(role="dialog", name="New action"), "New action"
    )


def test_action_editor_tabs_validate_save_and_reopen(strata):
    _open_new_action(strata)

    def editor(title="New action"):
        return strata.wait(
            lambda: strata.window.find(role="dialog", name=title), title
        )

    dialog = editor()

    def control(name, role=None):
        return strata.wait(lambda: dialog.find(name=name, role=role), name)

    def tab(name):
        strata.pointer.click(control(name, "page tab"))
        strata.wait(
            lambda: control(name, "page tab").has_state("selected"), f"{name} tab"
        )

    def fill(name, value):
        node = control(name, "text")
        strata.pointer.click(node)
        strata.keyboard.press("ctrl+a")
        for index, line in enumerate(value.split("\n")):
            if index:
                strata.keyboard.press("Return")
            strata.keyboard.type_text(line)
        strata.wait(lambda: node.text == value, f"{name} text")

    def click_backdrop():
        bounds = strata.window.screen_bounds()
        strata.pointer.click(strata.window, at=(bounds.x + 5, bounds.y + 5))

    fill("Name", "Batch rename")
    fill("Description", "Process selected files")
    click_backdrop()
    strata.pointer.click(control("scissors icon", "toggle button"))
    tab("Script")
    strata.pointer.click(control("Command", "toggle button"))
    fill("Program", "printf")
    fill("Arguments", "%s\\n\n{paths}")
    assert dialog.alive, "Enter in the multiline editor must not submit the form"

    tab("Behavior")
    assert not control("Stop", "toggle button").has_state("sensitive")
    strata.pointer.click(control("Per item", "toggle button"))
    strata.wait(
        lambda: control("Stop", "toggle button").has_state("sensitive"),
        "per-item failure policy to become available",
    )
    strata.pointer.click(control("Stop", "toggle button"))
    strata.pointer.click(control("Menu item", "toggle button"))
    fill("Extensions", "txt")
    strata.pointer.click(control("Create action", "button"))
    strata.wait(
        lambda: control("Script", "page tab").has_state("selected"),
        "invalid argument token to return to Script",
    )
    action_dir = strata.environment.config_home / "strata/actions/batch-rename"
    assert not action_dir.exists(), "invalid drafts must not write an action"
    fill("Arguments", "%s\\n\n{path}")
    strata.pointer.click(control("Create action", "button"))
    manifest = action_dir / "action.toml"
    strata.wait(manifest.exists, "the action manifest")
    strata.wait(
        lambda: strata.window.find(role="dialog", name="New action") is None,
        "the saved editor to close",
    )
    saved = tomllib.loads(manifest.read_text())
    assert saved["name"] == "Batch rename"
    assert saved["description"] == "Process selected files"
    assert saved["icon"] == "scissors"
    assert saved["menu"] == "top"
    assert saved["when"]["extensions"] == ["txt"]
    assert saved["run"]["runtime"] == "command"
    assert saved["run"]["program"] == "printf"
    assert saved["run"]["args"] == ["%s\\n", "{path}"]
    assert saved["run"]["mode"] == "per-item"
    assert saved["run"]["on_error"] == "stop"

    assert strata.window.find(role="button", name="Edit").activate()
    dialog = editor("Edit action")
    assert control("Id", "text").text == "batch-rename"
    assert not control("Id", "text").has_state("editable")
    tab("Script")
    assert control("Arguments", "text").text == "%s\\n\n{path}"
    click_backdrop()
    fill("Program", "false")
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="dialog", name="Edit action") is None,
        "Escape to discard the edit",
    )
    assert tomllib.loads(manifest.read_text()) == saved

    assert strata.window.find(role="button", name="Close settings").activate()
    strata.wait(
        lambda: strata.window.find(role="button", name="Close settings") is None,
        "Settings to close",
    )
    strata.open_context_menu("todo.txt")
    assert "Batch rename" in strata.menu_items()
    strata.dismiss_menu()


def test_script_library_previews_preserves_cancelled_edits_and_runs_a_recipe(strata):
    editor = _open_new_action(strata)
    strata.keyboard.type_text("Checksum job")
    strata.pointer.click(editor.find(role="page tab", name="Script"))
    script = strata.wait(lambda: editor.find(role="text", name="Script"), "script editor")
    strata.pointer.click(script)
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("print('my draft')")
    strata.wait(lambda: script.text == "print('my draft')", "custom draft")

    def library():
        assert editor.find(role="button", name="Examples…").activate()
        return strata.wait(
            lambda: strata.window.find(role="dialog", name="Script examples"), "script library"
        )

    picker = library()
    assert picker.find(role="button", name="Replace script") is not None
    strata.pointer.click(picker.find(role="button", name="Cancel"))
    strata.wait(
        lambda: strata.window.find(role="dialog", name="Script examples") is None,
        "cancelled library to close",
    )
    assert script.text == "print('my draft')"
    picker = library()
    strata.pointer.click(picker.find(role="combo box"))
    strata.keyboard.press("End")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: picker.find(role="label", name="SHA-256 checksums"), "checksum example"
    )
    preview = picker.find(role="text", name="Example code").text
    strata.pointer.click(picker.find(role="button", name="Replace script"))
    strata.wait(
        lambda: strata.window.find(role="dialog", name="Script examples") is None,
        "chosen library to close",
    )
    assert script.text == preview
    strata.pointer.click(editor.find(role="page tab", name="Behavior"))
    menu_item = strata.wait(
        lambda: editor.find(role="toggle button", name="Menu item"), "placement control"
    )
    strata.pointer.click(menu_item)
    strata.pointer.click(editor.find(role="button", name="Create action"))
    action_dir = strata.environment.config_home / "strata/actions/checksum-job"
    strata.wait(lambda: (action_dir / "action.toml").exists(), "saved example")
    assert (action_dir / "main.py").read_text() == preview
    manifest = tomllib.loads((action_dir / "action.toml").read_text())
    assert manifest["name"] == "Checksum job"
    assert manifest["run"]["mode"] == "per-item"
    assert manifest["when"]["kinds"] == ["file"]
    strata.wait(
        lambda: strata.window.find(role="dialog", name="New action") is None,
        "editor to close",
    )
    assert strata.window.find(role="button", name="Close settings").activate()
    strata.wait(
        lambda: strata.window.find(role="button", name="Close settings") is None,
        "settings to close",
    )
    original = strata.fixture.path("todo.txt").read_bytes()
    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Checksum job")
    checksum = strata.fixture.path("todo.txt.sha256")
    strata.wait(checksum.exists, "the recipe to create a checksum")
    strata.wait(
        lambda: checksum.read_text().startswith(hashlib.sha256(original).hexdigest()),
        "the complete checksum",
    )
    assert strata.fixture.path("todo.txt").read_bytes() == original
