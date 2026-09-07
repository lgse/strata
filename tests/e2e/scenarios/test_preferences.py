# SPDX-License-Identifier: GPL-3.0-or-later
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
    folder_peeking=False, type_to_search=False, single_click_previews=False
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
    for label in ["Folder peeking", "Type to search", "Single-click file previews"]:
        assert not _switch(strata.window, label).has_state("checked")
