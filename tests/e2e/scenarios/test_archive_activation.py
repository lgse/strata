# SPDX-License-Identifier: MIT
import shutil
import zipfile
from pathlib import Path

import pytest

from harness.artifacts import ArtifactCollector
from harness.modes import ALL_MODES


@pytest.mark.preferences(
    list_file_clicks=2, grid_file_clicks=2, explorer_file_clicks=2,
)
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("activation", ["keyboard", "double-click"])
@pytest.mark.parametrize("format", ["zip", "rar"])
def test_archive_activation_extracts_to_subfolder(strata, mode, activation, format):
    strata.wait_for_focused_entry("archive")
    fixture = strata.fixture
    archive_name = f"activation.{format}"
    if format == "zip":
        with zipfile.ZipFile(fixture.path(archive_name), "w") as archive:
            archive.writestr("activated.txt", "extracted by activation\n")
        member, contents = "activated.txt", "extracted by activation\n"
    else:
        shutil.copyfile(Path(__file__).parents[2] / "fixtures/rar/version.rar", fixture.path(archive_name))
        member, contents = "VERSION", "unrar-0.4.0"
    strata.keyboard.press("F5")
    strata.entry(archive_name)

    if activation == "keyboard":
        strata.select_entry_with_keyboard(archive_name)
        strata.keyboard.press("Return")
    else:
        strata.double_click_entry(archive_name)

    subfolder = fixture.path("activation")
    extracted = subfolder / member
    strata.wait(lambda: extracted.exists(), "archive activation to extract into a subfolder")
    strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
    assert extracted.read_text() == contents
    assert fixture.path(archive_name).exists()
    assert strata.pane().name == fixture.root.name
    strata.entry("activation")
    if format == "rar":
        collector = ArtifactCollector(test_name=f"rar-activation-{mode}-{activation}")
        strata.screenshot(collector.directory / "after.png")
