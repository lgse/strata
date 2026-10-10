# SPDX-License-Identifier: MIT
"""Saved preferences and live controls share one application-wide state."""

import subprocess

import pytest

from harness.application import binary_path
from harness.environment import process_environment


def _switch(window, name):
    return next(
        (
            node
            for node in window.find_all(name=name, rendered=False)
            if node.role in {"check box", "toggle button", "switch"}
        ),
        None,
    )


def _open_settings(strata, window):
    button = window.find(role="button", name="Settings")
    assert button is not None and button.activate()
    strata.wait(lambda: _switch(window, "Folder peeking"), "General settings to exist")


@pytest.mark.preferences(
    folder_peeking=False, type_to_search=False, single_click_previews=False,
    columns_mirror_selection=False,
    filter_include_subfolders=False, open_folder_after_drop=False,
)
def test_preferences_sync_across_windows_and_restart(strata):
    variables = process_environment()
    variables.update(strata.environment.variables())
    variables.update(strata.display.environment)
    subprocess.run(
        [str(binary_path()), str(strata.fixture.root)],
        env=variables,
        cwd=strata.fixture.root,
        check=True,
        timeout=30,
        capture_output=True,
    )
    windows = strata.wait(
        lambda: (
            frames
            if len(frames := strata.application.application_node.find_all(
                role="frame", name="Strata"
            )) == 2
            else None
        ),
        "two windows in the same Strata application",
    )
    for window in windows:
        _open_settings(strata, window)
    for label, key in [
        ("Folder peeking", "folder_peeking"),
        ("Type to search", "type_to_search"),
        ("Single-click file previews", "single_click_previews"),
        ("Mirror columns selection", "columns_mirror_selection"),
        ("Include subfolders", "filter_include_subfolders"),
        ("Open folder after dropping files", "open_folder_after_drop"),
    ]:
        switches = [_switch(window, label) for window in windows]
        assert all(not toggle.has_state("checked") for toggle in switches)
        assert switches[0].activate()
        strata.wait(
            lambda: all(toggle.has_state("checked") for toggle in switches),
            f"{label} to enable in both windows",
        )
        assert switches[1].activate()
        strata.wait(
            lambda: all(not toggle.has_state("checked") for toggle in switches),
            f"{label} to disable in both windows",
        )
        strata.wait(
            lambda: strata.environment.read_preferences().get(key) == "false",
            f"{label} to be saved",
        )
    strata.application.stop()
    strata.application.start()
    _open_settings(strata, strata.window)
    for label in [
        "Folder peeking", "Type to search", "Single-click file previews",
        "Include subfolders", "Open folder after dropping files",
    ]:
        assert not _switch(strata.window, label).has_state("checked")


def _search_button(window):
    return window.find(role="button", name="Search (Ctrl+K)")


def _close_button(window):
    return window.find(role="button", name="Close window")


@pytest.mark.preferences(tenxer_mode=True)
def test_enabled_tenxer_applies_before_settings_and_survives_restart(strata):
    assert _search_button(strata.window) is None
    assert _close_button(strata.window) is not None
    strata.application.stop()
    strata.application.start()
    assert _search_button(strata.window) is None
    assert _close_button(strata.window) is not None
    assert strata.environment.read_preferences().get("tenxer_mode") == "true"


@pytest.mark.preferences(language="fr")
def test_language_selection_and_auto_detection_apply_after_relaunch(strata):
    settings = strata.wait(
        lambda: strata.window.find(role="button", name="Paramètres"),
        "saved French language before opening Settings",
    )
    strata.pointer.click(settings)
    language = strata.wait(
        lambda: strata.window.find(role="button", name="Langue"), "French language selector"
    )
    strata.pointer.click(language)
    japanese = strata.wait(
        lambda: strata.window.find(role="label", name="日本語"), "Japanese autonym"
    )
    strata.pointer.click(japanese)
    strata.wait(
        lambda: strata.environment.read_preferences().get("language") == '"ja"',
        "manual language saved",
    )
    strata.wait(
        lambda: strata.window.find(role="button", name="Redémarrer maintenant"),
        "restart notice remains in the running French language",
    )
    assert strata.window.find(role="button", name="Langue") is not None
    assert strata.window.find(role="button", name="言語") is None

    strata.application.stop()
    strata.application.start()
    settings = strata.wait(
        lambda: strata.window.find(role="button", name="設定"), "Japanese after relaunch"
    )
    strata.pointer.click(settings)
    language = strata.wait(
        lambda: strata.window.find(role="button", name="言語"), "Japanese language selector"
    )
    strata.pointer.click(language)
    automatic = strata.wait(
        lambda: strata.window.find(role="label", name="自動検出"), "Auto-detect in Japanese"
    )
    strata.pointer.click(automatic)
    strata.wait(
        lambda: strata.environment.read_preferences().get("language") == '"auto"',
        "Auto-detect saved",
    )
    assert strata.window.find(role="button", name="言語") is not None

    strata.application.stop()
    strata.application.start()
    strata.wait(
        lambda: strata.window.find(role="button", name="Settings"),
        "Auto-detect returns to the isolated C.UTF-8 environment's English UI",
    )


def _tab_to(strata, node):
    for _ in range(60):
        if node.has_state("focused"):
            return
        before = strata.focused_node()
        strata.keyboard.press("Tab")
        strata.wait(lambda: strata.focused_node() != before, "Tab to move focus", timeout=5)
    raise AssertionError(f"Tab never reached {node}")


@pytest.mark.preferences(folder_peeking=False)
def test_unsaved_changes_show_one_notice_and_return_focus(strata):
    settings = strata.environment.settings_path
    title = "Settings can't be saved"
    _open_settings(strata, strata.window)
    settings.unlink()
    settings.mkdir()

    def notice():
        return strata.window.find(role="dialog", name=title)

    assert notice() is None, "startup changes nothing"
    toggle = _switch(strata.window, "Folder peeking")
    _tab_to(strata, toggle)
    strata.keyboard.press("space")
    text = " ".join(
        node.text for _, node in strata.wait(notice, "the save notice").walk() if node.text
    )
    assert "Changes last only until Strata closes" in text
    assert str(settings) in text.replace("\n", "")
    assert toggle.has_state("checked")
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: notice() is None and toggle.has_state("focused"),
        "the notice to close with focus back on the switch",
    )

    strata.keyboard.press("space")
    strata.wait(lambda: not toggle.has_state("checked"), "the second change to apply")
    assert notice() is None, "one notice per failure streak"
    assert settings.is_dir()
