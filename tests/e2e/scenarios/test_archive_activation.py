# SPDX-License-Identifier: MIT
import zipfile

import pytest

from harness.modes import ALL_MODES


@pytest.mark.preferences(
    list_file_clicks=2, grid_file_clicks=2, explorer_file_clicks=2,
)
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("activation", ["keyboard", "double-click"])
def test_archive_activation_extracts_in_place(strata, mode, activation):
    fixture = strata.fixture
    with zipfile.ZipFile(fixture.path("activation.zip"), "w") as archive:
        archive.writestr("activated.txt", "extracted by activation\n")
    strata.keyboard.press("ctrl+r")
    strata.entry("activation.zip")

    if activation == "keyboard":
        strata.select_entry_with_keyboard("activation.zip")
        strata.keyboard.press("Return")
    else:
        strata.double_click_entry("activation.zip")

    extracted = fixture.path("activated.txt")
    strata.wait(lambda: extracted.exists(), "archive activation to extract in place")
    strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
    assert extracted.read_text() == "extracted by activation\n"
    assert fixture.path("activation.zip").exists()
    assert strata.pane().name == fixture.root.name
    strata.entry("activated.txt")
