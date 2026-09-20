# SPDX-License-Identifier: MIT
import io
import shutil
import tarfile
import zipfile
from pathlib import Path

import pytest

from harness.artifacts import ArtifactCollector



@pytest.mark.preferences(
    list_file_clicks=2, grid_file_clicks=2, explorer_file_clicks=2,
)
@pytest.mark.parametrize("activation", ["keyboard", "double-click"])
@pytest.mark.parametrize("format", ["zip", "tar.gz"])
def test_archive_activation_extracts_to_subfolder(strata, activation, format):
    strata.wait_for_focused_entry("archive")
    fixture = strata.fixture
    archive_name = f"activation.{format}"
    contents = "extracted by activation\n"
    members = [("activated.txt", contents.encode()), ("nested/second.txt", b"second member\n")]
    if format == "zip":
        with zipfile.ZipFile(fixture.path(archive_name), "w") as archive:
            for member, data in members:
                archive.writestr(member, data)
    else:
        with tarfile.open(fixture.path(archive_name), "w:gz") as archive:
            for member, data in members:
                info = tarfile.TarInfo(member)
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
    strata.entry(archive_name)

    if activation == "keyboard":
        strata.select_entry("todo.txt")
        strata.select_entry_with_keyboard(archive_name)
        strata.keyboard.press("Return")
    else:
        strata.double_click_entry(archive_name)

    subfolder = fixture.path("activation")
    extracted = subfolder / "activated.txt"
    strata.wait(lambda: extracted.exists(), "archive activation to bundle spilled members")
    strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
    assert extracted.read_text() == contents
    assert (subfolder / "nested/second.txt").read_bytes() == b"second member\n"
    assert not fixture.path("activated.txt").exists()
    assert not fixture.path("nested").exists()
    assert fixture.path(archive_name).exists()
    assert strata.pane().name == fixture.root.name
    strata.entry("activation")
    extracted.write_text("keep existing edits\n")
    for suffix in [1, 2]:
        if activation == "keyboard":
            strata.select_entry("todo.txt")
            strata.select_entry_with_keyboard(archive_name)
            strata.keyboard.press("Return")
        else:
            strata.double_click_entry(archive_name)
        fresh = fixture.path(f"activation ({suffix})") / "activated.txt"
        strata.wait(lambda: fresh.exists(), "repeated activation to use a fresh folder")
        strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
        assert fresh.read_text() == contents
        assert extracted.read_text() == "keep existing edits\n"
        strata.entry(f"activation ({suffix})")


@pytest.mark.preferences(
    list_file_clicks=2, grid_file_clicks=2, explorer_file_clicks=2,
)
@pytest.mark.parametrize("activation", ["keyboard", "double-click"])
def test_archive_activation_extracts_a_single_root_archive_verbatim(strata, activation):
    strata.wait_for_focused_entry("archive")
    fixture = strata.fixture
    archive_name = "activation.rar"
    shutil.copyfile(Path(__file__).parents[2] / "fixtures/rar/version.rar", fixture.path(archive_name))
    strata.entry(archive_name)

    if activation == "keyboard":
        strata.select_entry("todo.txt")
        strata.select_entry_with_keyboard(archive_name)
        strata.keyboard.press("Return")
    else:
        strata.double_click_entry(archive_name)

    extracted = fixture.path("VERSION")
    strata.wait(lambda: extracted.exists(), "single-root archive to extract verbatim")
    strata.wait(lambda: strata.dialog() is None, "extraction progress dismissal")
    assert extracted.read_text() == "unrar-0.4.0"
    assert not fixture.path("activation").exists()
    assert fixture.path(archive_name).exists()
    collector = ArtifactCollector(test_name=f"rar-activation-{activation}")
    strata.screenshot(collector.directory / "after.png")
