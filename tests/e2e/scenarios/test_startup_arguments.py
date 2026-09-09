# SPDX-License-Identifier: MIT
"""Shell arguments reach every requested location without changing path bytes."""

import os
import subprocess

from harness.application import binary_path
from harness.environment import process_environment


def test_multiple_arguments_include_non_utf8_directory_and_file(strata):
    root = os.fsencode(strata.fixture.root)
    directories = [root + b"/startup-first", root + b"/startup-\xff"]
    markers = ["first-argument.txt", "non-utf8-argument.txt"]
    for directory, marker in zip(directories, markers):
        os.mkdir(directory)
        with open(directory + b"/" + marker.encode(), "wb") as stream:
            stream.write(b"startup regression\n")

    file_argument = root + b"/reveal-me.txt"
    with open(file_argument, "wb") as stream:
        stream.write(b"reveal regression\n")

    variables = process_environment()
    variables.update(strata.environment.variables())
    variables.update(strata.display.environment)
    subprocess.run(
        [os.fsencode(binary_path()), *directories, file_argument],
        env=variables,
        cwd=strata.fixture.root,
        check=True,
        timeout=30,
        capture_output=True,
    )

    def requested_windows_exist():
        windows = strata.application.application_node.find_all(role="frame", name="Strata")
        if len(windows) != 4:
            return False
        return all(
            any(window.find(name=marker) is not None for window in windows)
            for marker in markers
        ) and any(
            node.has_state("selected")
            for window in windows
            for node in window.find_all(name="reveal-me.txt")
        )

    strata.wait(
        requested_windows_exist,
        "one populated window per argument, with the file selected in its parent",
    )
