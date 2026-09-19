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


def test_script_library_filters_preserves_drafts_and_shows_finished_jobs(strata):
    editor = _open_new_action(strata)
    strata.keyboard.type_text("Checksum job")
    strata.pointer.click(editor.find(role="page tab", name="Script"))
    script = strata.wait(lambda: editor.find(role="text", name="Script"), "script editor")
    strata.pointer.click(script)
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("print('my draft')")
    strata.wait(lambda: script.text == "print('my draft')", "custom draft")

    def library():
        strata.pointer.click(editor.find(name="Library"))
        return strata.wait(
            lambda: strata.window.find(name="Script library"), "library dropdown"
        )

    picker = library()
    strata.pointer.click(picker.find(role="toggle button", name="Media"))
    strata.wait(
        lambda: picker.find(role="button", name="SHA-256 checksums", states=["visible"], rendered=False) is None,
        "Files templates excluded from Media",
    )
    search = picker.find(role="text", name="Search templates")
    strata.pointer.click(search)
    strata.keyboard.type_text("FFMPEG MP4")
    strata.wait(lambda: picker.find(role="button", name="Convert videos to MP4"), "media search")
    strata.wait(
        lambda: picker.find(role="button", name="Extract MP3 audio", states=["visible"], rendered=False) is None,
        "search excludes other media templates",
    )
    strata.keyboard.press("Down")
    strata.wait(
        lambda: picker.find(role="button", name="Convert videos to MP4", states=["focused"]),
        "keyboard focus moves from search to its result",
    )
    strata.keyboard.press("Return")
    keep = strata.wait(lambda: picker.find(role="button", name="Keep draft"), "keyboard selection")
    strata.pointer.click(keep)
    assert script.text == "print('my draft')"
    strata.pointer.click(search)
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("unmatched template")
    strata.wait(lambda: picker.find(role="label", name="No templates match your search."), "empty search")
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.window.find(name="Script library") is None, "library to close")
    assert strata.window.find(role="dialog", name="New action") is not None
    assert script.text == "print('my draft')"
    picker = library()
    strata.pointer.click(picker.find(role="toggle button", name="Files"))
    search = picker.find(role="text", name="Search templates")
    strata.pointer.click(search)
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("sha-256")
    strata.keyboard.press("Return")
    replace = strata.wait(lambda: picker.find(role="button", name="Replace script"), "inline replacement choice")
    assert script.text == "print('my draft')"
    action_dir = strata.environment.config_home / "strata/actions/checksum-job"
    assert not action_dir.exists(), "search Enter must not save the action"
    strata.pointer.click(picker.find(role="button", name="Keep draft"))
    assert script.text == "print('my draft')"
    strata.pointer.click(picker.find(role="button", name="SHA-256 checksums"))
    replace = strata.wait(lambda: picker.find(role="button", name="Replace script"), "replacement choice")
    strata.pointer.click(replace)
    strata.wait(lambda: strata.window.find(name="Script library") is None, "chosen library to close")
    applied = script.text
    assert applied != "print('my draft')"
    strata.pointer.click(editor.find(role="page tab", name="Behavior"))
    menu_item = strata.wait(
        lambda: editor.find(role="toggle button", name="Menu item"), "placement control"
    )
    strata.pointer.click(menu_item)
    strata.pointer.click(editor.find(role="button", name="Create action"))
    action_dir = strata.environment.config_home / "strata/actions/checksum-job"
    strata.wait(lambda: (action_dir / "action.toml").exists(), "saved example")
    assert (action_dir / "main.py").read_text() == applied
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
    finished = strata.wait(lambda: strata.window.find(name="1 job finished"), "finished job indicator")
    strata.pointer.click(finished)
    strata.wait(lambda: strata.window.find(role="label", name="Completed"), "completed row")
    strata.pointer.click(strata.window.find(role="button", name="Details"))
    strata.wait(lambda: strata.window.find(role="label", name_matches="Created .*todo.txt.sha256"), "finished job output")
    strata.pointer.click(strata.window.find(role="button", name="Hide"))
    strata.wait(lambda: strata.window.find(role="label", name_matches="Created .*todo.txt.sha256") is None, "hidden output")
    strata.pointer.click(strata.window.find(role="button", name="Minimize"))
    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Checksum job")
    finished = strata.wait(lambda: strata.window.find(name="2 jobs finished · failures"), "failed rerun in history")
    strata.pointer.click(finished)
    strata.wait(lambda: strata.window.find(role="label", name="Failed"), "failed job row")
    assert strata.window.find(role="label", name="Completed") is not None
    strata.pointer.click(strata.window.find(role="button", name="Details"))
    strata.wait(lambda: strata.window.find(role="label", name_matches="FileExistsError"), "failure details")
    strata.pointer.click(strata.window.find(role="button", name="Dismiss"))
    strata.wait(lambda: strata.window.find(name="1 job finished"), "remaining completed history")
    strata.pointer.click(strata.window.find(role="button", name="Clear finished"))
    strata.wait(lambda: strata.window.find(name="1 job finished") is None, "cleared history")
