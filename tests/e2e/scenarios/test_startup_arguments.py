# SPDX-License-Identifier: MIT
"""Shell arguments reach every requested location without changing path bytes."""

import os
import subprocess

from harness.application import binary_path
from harness.environment import process_environment


def test_multiple_arguments_include_non_utf8_directory(strata):
    root = os.fsencode(strata.fixture.root)
    directories = [root + b"/startup-first", root + b"/startup-\xff"]
    markers = ["first-argument.txt", "non-utf8-argument.txt"]
    for directory, marker in zip(directories, markers):
        os.mkdir(directory)
        with open(directory + b"/" + marker.encode(), "wb") as stream:
            stream.write(b"startup regression\n")

    variables = process_environment()
    variables.update(strata.environment.variables())
    variables.update(strata.display.environment)
    subprocess.run(
        [os.fsencode(binary_path()), *directories],
        env=variables,
        cwd=strata.fixture.root,
        check=True,
        timeout=30,
        capture_output=True,
    )

    def requested_windows_exist():
        windows = strata.application.application_node.find_all(role="frame", name="Strata")
        if len(windows) != 3:
            return False
        return all(
            any(window.find(name=marker) is not None for window in windows)
            for marker in markers
        )

    strata.wait(requested_windows_exist, "one populated window for each path argument")
