# SPDX-License-Identifier: MIT

import tomllib

from harness.tree import Atspi


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
    name = control("Name", "text")
    strata.wait(lambda: name.has_state("focused"), "Name focused on opening the editor")
    text = Atspi.Accessible.get_text_iface(name.accessible)
    assert Atspi.Text.get_n_selections(text) == 0
    assert Atspi.Text.get_caret_offset(text) == len(saved["name"])
    strata.keyboard.type_text(" edited")
    strata.wait(lambda: name.text == saved["name"] + " edited", "typing appends to the action name")
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


def test_batch_rename_template_confirms_before_renaming_and_opens_jobs(strata):
    editor = _open_new_action(strata)
    strata.pointer.click(editor.find(role="page tab", name="Script"))
    strata.pointer.click(editor.find(name="Library"))
    picker = strata.wait(lambda: strata.window.find(name="Script library"), "library")
    strata.pointer.click(picker.find(role="text", name="Search templates"))
    strata.keyboard.type_text("rename")
    strata.pointer.click(strata.wait(lambda: picker.find(role="button", name="Batch rename"), "rename template"))
    strata.wait(lambda: strata.window.find(name="Script library") is None, "applied template")
    strata.pointer.click(editor.find(role="page tab", name="Behavior"))
    strata.pointer.click(editor.find(role="toggle button", name="Menu item"))
    confirm = next(node for node in editor.find_all(name="Confirm") if node.role in {"check box", "toggle button", "switch"})
    strata.pointer.click(confirm)
    strata.pointer.click(editor.find(role="button", name="Create action"))
    strata.wait(lambda: strata.window.find(role="dialog", name="New action") is None, "saved template")
    assert strata.window.find(role="button", name="Close settings").activate()
    strata.wait(lambda: strata.window.find(role="button", name="Close settings") is None, "settings closed")
    originals = {name: strata.fixture.path(name).read_bytes() for name in ("readme.md", "todo.txt")}
    strata.select_entry("readme.md")
    strata.click_entry_with("todo.txt", ["ctrl"])
    strata.wait(lambda: set(strata.selected_names()) == set(originals), "both files selected")
    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Batch rename")
    confirmation = strata.wait(lambda: strata.window.find(role="dialog", name="Run this action?"), "confirmation")
    assert strata.window.find(role="button", name="Minimize") is None
    strata.pointer.click(confirmation.find(role="button", name="Cancel"))
    strata.wait(lambda: strata.window.find(role="dialog", name="Run this action?") is None, "cancelled confirmation")
    for name, contents in originals.items():
        assert strata.fixture.path(name).read_bytes() == contents
    assert strata.window.find(name="1 job finished") is None
    strata.open_context_menu("todo.txt")
    strata.choose_menu_item("Batch rename")
    confirmation = strata.wait(lambda: strata.window.find(role="dialog", name="Run this action?"), "confirmation")
    strata.pointer.click(confirmation.find(role="button", name="Run"))
    strata.wait(lambda: strata.window.find(role="label", name_matches="Done in "), "automatically opened rename result")
    assert strata.window.find(role="dialog", name="Run this action?") is None
    strata.pointer.click(strata.window.find(role="button", name="Details"))
    strata.wait(lambda: strata.window.find(role="label", name_matches="001_readme.md"), "rename output")
    for index, (name, contents) in enumerate(originals.items(), start=1):
        renamed = f"{index:03d}_{name}"
        assert strata.fixture.path(renamed).read_bytes() == contents
        assert not strata.fixture.path(name).exists()
