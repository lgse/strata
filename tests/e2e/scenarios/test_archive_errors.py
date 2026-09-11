# SPDX-License-Identifier: MIT
import os
import shutil
import struct
import zipfile
import zlib
from pathlib import Path

import pytest

from harness.artifacts import ArtifactCollector


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


def test_cancelled_extract_to_does_not_hijack_later_extract_here(strata):
    fixture = strata.fixture
    archive_name = "content-encrypted.7z"
    shutil.copyfile(ARCHIVE_FIXTURES / archive_name, fixture.path(archive_name))
    with zipfile.ZipFile(fixture.path("later.zip"), "w") as archive:
        archive.writestr("later.txt", "later extraction\n")
    strata.keyboard.press("ctrl+r")
    strata.open_context_menu(archive_name)
    strata.choose_menu_item("Extract to…")
    destination = fixture.path("leftover")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(destination))
    strata.wait(lambda: field.text == str(destination), "the destination field")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: (dialog := strata.dialog()) is not None and dialog.name == "Extract",
        "the password prompt",
    )
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "password prompt cancellation")
    assert not destination.exists()
    strata.open_context_menu("later.zip")
    strata.choose_menu_item("Extract here")
    strata.wait(lambda: fixture.path("later.txt").exists(), "later extraction")
    strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
    assert strata.current_directory() == fixture.root.name
    assert fixture.path("later.txt").read_text() == "later extraction\n"
    assert not destination.exists()
    strata.entry("later.txt")
    collector = ArtifactCollector(test_name="cancelled-extract-to")
    strata.screenshot(collector.directory / "after.png")


_CRCTABLE = None


def _zipcrypto_crc32(ch, crc):
    global _CRCTABLE
    if _CRCTABLE is None:
        table = []
        for value in range(256):
            for _ in range(8):
                value = (value >> 1) ^ 0xEDB88320 if value & 1 else value >> 1
            table.append(value)
        _CRCTABLE = table
    return (crc >> 8) ^ _CRCTABLE[(crc ^ ch) & 0xFF]


def _zipcrypto_encrypt(password, data):
    key0, key1, key2 = 305419896, 591751049, 878082192

    def update(byte):
        nonlocal key0, key1, key2
        key0 = _zipcrypto_crc32(byte, key0)
        key1 = (key1 + (key0 & 0xFF)) & 0xFFFFFFFF
        key1 = (key1 * 134775813 + 1) & 0xFFFFFFFF
        key2 = _zipcrypto_crc32(key1 >> 24, key2)

    for byte in password:
        update(byte)
    out = bytearray()
    for byte in data:
        key = key2 | 2
        out.append(byte ^ (((key * (key ^ 1)) >> 8) & 0xFF))
        update(byte)
    return bytes(out)


def _raw_deflate(data):
    compressor = zlib.compressobj(level=9, wbits=-15)
    return compressor.compress(data) + compressor.flush()


def _write_zipcrypto(path, password, name, contents, *, deflated=False):
    crc = zlib.crc32(contents) & 0xFFFFFFFF
    payload = _raw_deflate(contents) if deflated else contents
    header = os.urandom(11) + bytes([(crc >> 24) & 0xFF])
    encrypted = _zipcrypto_encrypt(password, header + payload)
    name_b = name.encode("utf-8")
    flags = 0x0001
    method = 8 if deflated else 0
    local = struct.pack(
        "<IHHHHHIIIHH",
        0x04034B50,
        20,
        flags,
        method,
        0,
        0,
        crc,
        len(encrypted),
        len(contents),
        len(name_b),
        0,
    )
    local_data = local + name_b + encrypted
    central = struct.pack(
        "<IHHHHHHIIIHHHHHII",
        0x02014B50,
        20,
        20,
        flags,
        method,
        0,
        0,
        crc,
        len(encrypted),
        len(contents),
        len(name_b),
        0,
        0,
        0,
        0,
        0,
        0,
    )
    cd = central + name_b
    eocd = struct.pack(
        "<IHHHHIIH",
        0x06054B50,
        0,
        0,
        1,
        1,
        len(cd),
        len(local_data),
        0,
    )
    path.write_bytes(local_data + cd + eocd)


def _zipcrypto_crc_collision(path, member="some.txt"):
    for candidate in range(4096):
        password = str(candidate).encode()
        try:
            with zipfile.ZipFile(path) as archive:
                archive.read(member, pwd=password)
        except RuntimeError as error:
            if "Bad password" in str(error):
                continue
            return str(candidate)
        except (zipfile.BadZipFile, OSError, zlib.error):
            return str(candidate)
    raise AssertionError("no ZipCrypto CRC collision in 0..4096")


@pytest.mark.parametrize(
    "deflated,contents",
    [
        pytest.param(False, b"hello from zipcrypto", id="stored"),
        pytest.param(True, b"hello from zipcrypto\n" * 64, id="deflated"),
    ],
)
def test_zipcrypto_collision_reopens_extract_dialog(strata, deflated, contents):
    fixture = strata.fixture
    archive_name = "password.zip"
    archive_path = fixture.path(archive_name)
    _write_zipcrypto(
        archive_path, b"zipsecret", "some.txt", contents, deflated=deflated
    )
    collision = _zipcrypto_crc_collision(archive_path)
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

    strata.keyboard.type_text(collision)
    strata.pointer.click(strata.dialog_button("Extract"))

    dialog = strata.wait(
        lambda: (
            dialog
            if (dialog := strata.dialog()) is not None
            and dialog.name == "Extract"
            and dialog.find(role="password text", states={"focused"}) is not None
            else None
        ),
        "the password dialog to reopen after a ZipCrypto CRC collision",
    )
    assert dialog.find(role="label", name="Invalid password") is not None
    assert dialog.find(role="label", name="Unable to complete operation") is None
    assert dialog.find(role="label", name="This file is not a valid archive or is damaged.") is None

    strata.keyboard.type_text("zipsecret")
    strata.pointer.click(strata.dialog_button("Extract"))
    extracted = fixture.path("some.txt")
    strata.wait(lambda: extracted.exists(), "the archive to extract with the correct password")
    assert extracted.read_text() == contents.decode()
    strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
