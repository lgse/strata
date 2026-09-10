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
    broken_link = root + b"/broken-link.txt"
    os.symlink(root + b"/missing-target.txt", broken_link)

    variables = process_environment()
    variables.update(strata.environment.variables())
    variables.update(strata.display.environment)
    subprocess.run(
        [os.fsencode(binary_path()), *directories, file_argument, broken_link],
        env=variables,
        cwd=strata.fixture.root,
        check=True,
        timeout=30,
        capture_output=True,
    )

    def requested_windows_exist():
        windows = strata.application.application_node.find_all(role="frame", name="Strata")
        if len(windows) != 5:
            return False
        return all(
            any(window.find(name=marker) is not None for window in windows)
            for marker in markers
        ) and all(
            any(
                node.has_state("selected")
                for window in windows
                for node in window.find_all(name=name)
            )
            for name in ["reveal-me.txt", "broken-link.txt"]
        )

    strata.wait(
        requested_windows_exist,
        "one window per argument, with the file and broken symlink revealed",
    )
