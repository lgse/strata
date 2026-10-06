# SPDX-License-Identifier: MIT

import os
import subprocess
from urllib.parse import quote_from_bytes

import pytest

from harness.environment import process_environment

COLLIDING_NAME = "bad�name.txt"


def call_file_manager(strata, method, path):
    variables = process_environment()
    variables.update(strata.environment.variables())
    variables.update(strata.display.environment)
    subprocess.run(
        [
            "dbus-send",
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.FileManager1",
            "/org/freedesktop/FileManager1",
            f"org.freedesktop.FileManager1.{method}",
            f"array:string:file://{quote_from_bytes(path)}",
            "string:",
        ],
        env=variables,
        check=True,
        timeout=30,
        capture_output=True,
    )


@pytest.mark.parametrize("method", ["ShowItems", "ShowItemProperties"])
def test_file_manager_reveals_only_the_requested_colliding_name(strata, method):
    documents = os.fsencode(strata.fixture.path("documents"))
    requested = documents + b"/bad\xe8name.txt"
    with open(requested, "wb") as stream:
        stream.write(b"requested\n")
    with open(documents + b"/bad\xe9name.txt", "wb") as stream:
        stream.write(b"the unrequested sibling\n")
    first_window = strata.window

    call_file_manager(strata, method, requested)

    def revealed_window():
        windows = strata.application.application_node.find_all(role="frame", name="Strata")
        return next(
            (
                window
                for window in windows
                if window != first_window
                and any(
                    node.has_state("selected")
                    for node in window.find_all(role="list item", name=COLLIDING_NAME)
                )
            ),
            None,
        )

    window = strata.wait(revealed_window, "a new window revealing the requested file")
    colliding = window.find_all(role="list item", name=COLLIDING_NAME)
    assert len(colliding) == 2
    selected = [node for node in colliding if node.has_state("selected")]
    assert len(selected) == 1
    if method == "ShowItemProperties":
        strata.wait(
            lambda: (dialog := window.find(role="dialog"))
            and dialog.find(role="label", name="10 B"),
            "Properties for the requested file only",
        )
    else:
        assert window.find(role="dialog") is None
