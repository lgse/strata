# SPDX-License-Identifier: MIT

import pytest

from harness.artifacts import ArtifactCollector
from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("destination", ["Home", "Trash"])
def test_sidebar_destinations_accept_three_files(strata, mode, destination):
    names = [f"drop-{letter}.txt" for letter in "abc"]
    for name in names:
        strata.fixture.path(name).write_text(name)
        strata.entry(name)
    strata.select_entry(names[0])
    for name in names[1:]:
        strata.pointer.click(strata.entry(name), modifiers=["ctrl"])
    strata.wait_for_selection(names)
    target = strata.sidebar_button(destination)
    source = strata.entry(names[0])
    strata.pointer.drag_points(
        strata.pointer.drag_origin(source), target.screen_bounds().center, release=False
    )
    try:
        artifacts = ArtifactCollector(test_name=f"sidebar-drop-{destination}-{mode}")
        strata.screenshot(artifacts.directory / "hover.png")
    finally:
        strata.pointer.connection.button(1, False)
    strata.wait(
        lambda: all(not strata.fixture.path(name).exists() for name in names),
        "all three sources to leave their original directory",
    )
    directory = strata.environment.home if destination == "Home" else strata.environment.trash_files
    strata.wait(
        lambda: all((directory / name).exists() and (directory / name).read_text() == name for name in names),
        "all three files to arrive intact",
    )
    if destination == "Trash":
        strata.keyboard.press("ctrl+z")
        strata.wait(
            lambda: all(strata.fixture.path(name).exists() for name in names),
            "all three trashed files to be restored by undo",
        )
