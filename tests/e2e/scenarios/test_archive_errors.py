# SPDX-License-Identifier: MIT
import zipfile

import pytest


@pytest.mark.parametrize("name", ["fake.zip", "fake.7z", "fake.tar", "fake.tar.gz"])
def test_invalid_archive_reports_damage_and_allows_another_extraction(strata, name):
    fixture = strata.fixture
    fixture.path(name).write_bytes(b"This is harmless text, not an archive.\n")
    with zipfile.ZipFile(fixture.path("valid.zip"), "w") as archive:
        archive.writestr("extracted.txt", "harmless contents")
    strata.keyboard.press("ctrl+r")
    strata.pointer.right_click(strata.entry(name))
    strata.choose_menu_item("Extract here")
    dialog = strata.wait(
        lambda: (
            dialog
            if (dialog := strata.dialog()) is not None
            and dialog.name == "Unable to complete operation"
            else None
        ),
        "the archive error dialog to replace the progress dialog",
    )
    assert dialog.find(role="label", name="This file is not a valid archive or is damaged.")
    assert not strata.window.find(role="progress bar")
    assert fixture.path(name).read_bytes() == b"This is harmless text, not an archive.\n"
    strata.pointer.click(strata.dialog_button("Close"))
    strata.wait(lambda: strata.dialog() is None, "error dismissal")
    strata.pointer.right_click(strata.entry("valid.zip"))
    strata.choose_menu_item("Extract here")
    strata.wait(lambda: fixture.path("extracted.txt").exists(), "valid archive extraction")
    assert fixture.path("extracted.txt").read_text() == "harmless contents"
    strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
