# SPDX-License-Identifier: MIT
"""Network discovery states, remote address errors, and saved connections.

The pinned image runs with GIO_USE_VFS=local, so GVfs discovery and remote
backends are unavailable; connecting reports missing protocol support.
"""

from __future__ import annotations

import json

import pytest


def _label_containing(strata, text: str):
    return strata.window.find(role="label", name_matches=f".*{text}.*")


def _open_address(strata, address: str):
    strata.keyboard.press("ctrl+l")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(address)
    strata.wait(lambda: field.text == address, "the address to be typed")
    strata.keyboard.press("Return")
    return strata.wait_for_dialog()


def _dialog_text(dialog) -> str:
    return "\n".join(node.name for node in dialog.find_all(role="label"))


def test_missing_discovery_and_protocol_support_keep_direct_entry_actionable(strata):
    strata.pointer.click(strata.sidebar_button("Network"))
    hint = strata.wait(
        lambda: _label_containing(strata, "Network discovery isn"),
        "the Network column to explain that discovery is unavailable",
    )
    assert "Ctrl+L" in hint.name, hint.name
    assert strata.dialog() is None, "discovery problems stay in the column"

    secret_host = "private-files.invalid"
    dialog = _open_address(strata, f"sftp://alice@{secret_host}:2222/srv/private")
    text = _dialog_text(dialog)
    assert "sftp://" in text and "isn't installed" in text, text
    assert secret_host not in text and "alice" not in text, text
    strata.pointer.click(strata.dialog_button("Close"))
    strata.wait(lambda: strata.dialog() is None, "the error to close")

    dialog = _open_address(strata, "https://cloud.invalid/remote.php/dav")
    text = _dialog_text(dialog)
    assert "Web addresses aren't file locations" in text and "davs://" in text, text
    strata.pointer.click(strata.dialog_button("Close"))
    strata.wait(lambda: strata.dialog() is None, "the error to close")


@pytest.fixture
def saved_connections(test_environment):
    """A connection written by a newer build: an unknown field must survive edits."""

    directory = test_environment.config_home / "strata"
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / "connections.json"
    path.write_text(
        json.dumps(
            {
                "version": 1,
                "connections": [
                    {
                        "id": "backups",
                        "name": "Backups",
                        "protocol": "sftp",
                        "uri": "sftp://backup@nas.invalid:2222/srv/backups",
                        "colour": "teal",
                    }
                ],
            }
        )
    )
    return path


def test_saved_connections_are_added_renamed_and_removed(saved_connections, strata):
    strata.wait(
        lambda: strata.window.find(role="label", name="CONNECTIONS"),
        "the Connections section to load saved connections at startup",
    )
    strata.sidebar_button("Backups")

    strata.pointer.click(strata.window.find(role="button", name="Add connection"))
    dialog = strata.wait_for_dialog()
    assert dialog.name == "Add connection", dialog.name
    assert dialog.find(role="password text") is None, "no password field"
    strata.pointer.click(dialog.find(role="toggle button", name_matches="FTPS.*"))
    strata.pointer.click(dialog.find(role="text", name="Server"))
    strata.keyboard.type_text("ftp.invalid")
    strata.pointer.click(dialog.find(role="text", name="Remote path (optional)"))
    strata.keyboard.type_text("/pub")
    strata.pointer.click(strata.dialog_button("Save"))
    strata.wait(lambda: strata.dialog() is None, "the form to close")
    added = strata.wait(
        lambda: strata.window.find(role="button", name="pub on ftp.invalid"),
        "the new connection row",
    )
    assert added is not None

    strata.pointer.right_click(strata.sidebar_button("Backups"))
    strata.choose_menu_item("Rename…")
    dialog = strata.wait_for_dialog()
    strata.pointer.click(dialog.find(role="text", name="Name"))
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("Offsite")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: strata.window.find(role="button", name="Offsite"),
        "the renamed row",
    )

    strata.pointer.right_click(strata.sidebar_button("pub on ftp.invalid"))
    strata.choose_menu_item("Remove…")
    strata.pointer.click(strata.dialog_button("Remove"))
    strata.wait(
        lambda: strata.window.find(role="button", name="pub on ftp.invalid") is None,
        "the removed row to disappear",
    )

    stored = json.loads(saved_connections.read_text())
    assert [entry["name"] for entry in stored["connections"]] == ["Offsite"]
    assert stored["connections"][0]["colour"] == "teal", "unknown fields survive"
    assert stored["connections"][0]["uri"] == "sftp://backup@nas.invalid:2222/srv/backups"
    assert "password" not in saved_connections.read_text()
