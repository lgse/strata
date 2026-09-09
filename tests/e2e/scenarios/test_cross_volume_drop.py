# SPDX-License-Identifier: MIT
import tempfile
from pathlib import Path

import pytest

from harness.modes import ALL_MODES


@pytest.fixture
def drop_volume(strata):
    with tempfile.TemporaryDirectory(prefix="strata-drop-volume-", dir="/dev/shm") as directory:
        volume = Path(directory)
        assert volume.stat().st_dev != strata.fixture.path("todo.txt").stat().st_dev
        strata.fixture.path("drop-volume").symlink_to(volume, target_is_directory=True)
        yield volume


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("action,tabs", [("Copy", 0), ("Move", 1), ("Cancel", 2)])
@pytest.mark.preferences(cross_volume_drop_strategy="always-ask")
def test_enter_activates_the_focused_cross_volume_choice(strata, mode, drop_volume, action, tabs):
    contents = strata.fixture.path("todo.txt").read_bytes()
    strata.pointer.drag(strata.entry("todo.txt"), strata.entry("drop-volume"))
    strata.wait_for_dialog()
    for _ in range(tabs):
        strata.keyboard.press("shift+Tab")
    strata.wait(lambda: strata.dialog_button(action).has_state("focused"), f"{action} focused")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.dialog() is None, "the copy-or-move dialog to close")
    source = strata.fixture.path("todo.txt")
    destination = drop_volume / "todo.txt"
    if action == "Cancel":
        assert not destination.exists(), "Cancel must not start a transfer"
        assert source.exists()
    else:
        strata.wait(lambda: destination.exists(), "the destination to be created")
        if action == "Move":
            strata.wait(lambda: not source.exists(), "the source to move")
        else:
            assert source.exists(), "Copy must preserve the source"
        assert destination.read_bytes() == contents


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.preferences(cross_volume_drop_strategy="always-copy")
def test_plain_cross_volume_copy_does_not_prompt_or_remove_source(strata, mode, drop_volume):
    source = strata.fixture.path("todo.txt")
    contents = source.read_bytes()
    strata.pointer.drag(strata.entry("todo.txt"), strata.entry("drop-volume"))
    destination = drop_volume / "todo.txt"
    strata.wait(lambda: destination.exists(), "the cross-device copy")
    assert source.read_bytes() == contents
    assert destination.read_bytes() == contents
    assert strata.dialog() is None
