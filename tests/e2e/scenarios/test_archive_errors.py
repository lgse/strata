# SPDX-License-Identifier: MIT
import shutil
import zipfile
from pathlib import Path

import pytest


ARCHIVE_FIXTURES = Path(__file__).parents[1] / "fixtures"


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


def test_wrong_extract_password_reopens_dialog_until_password_is_correct(strata):
    fixture = strata.fixture
    archive_name = "content-encrypted.7z"
    shutil.copyfile(ARCHIVE_FIXTURES / archive_name, fixture.path(archive_name))
    strata.keyboard.press("ctrl+r")
    strata.pointer.right_click(strata.entry(archive_name))
    strata.choose_menu_item("Extract here")

    dialog = strata.wait(
        lambda: (
            dialog
            if (dialog := strata.dialog()) is not None and dialog.name == "Extract"
            else None
        ),
        "the password dialog to replace the progress dialog",
    )
    strata.pointer.click(strata.dialog_button("Extract"))
    dialog = strata.wait_for_dialog()
    assert dialog.find(role="label", name="Enter a password") is not None

    strata.keyboard.type_text("wrong")
    strata.pointer.click(strata.dialog_button("Extract"))

    dialog = strata.wait(
        lambda: (
            dialog
            if (dialog := strata.dialog()) is not None
            and dialog.name == "Extract"
            and dialog.find(role="password text", states={"focused"}) is not None
            else None
        ),
        "the password dialog to reopen after the wrong password",
    )
    assert dialog.find(role="label", name="Invalid password") is not None
    assert dialog.find(role="label", name="Unable to complete operation") is None

    strata.keyboard.type_text("secret")
    strata.pointer.click(strata.dialog_button("Extract"))
    extracted = fixture.path("protected.txt")
    strata.wait(lambda: extracted.exists(), "the archive to extract with the correct password")
    assert extracted.read_text() == "password retry works\n"
    strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
