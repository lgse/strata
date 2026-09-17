# SPDX-License-Identifier: MIT
"""The embedded terminal runs a real shell across the PTY boundary."""

from __future__ import annotations

import time

# Long enough for a shell that should not exist to have written a file.
QUIET_PERIOD = 3.0


def _run(strata, command: str) -> None:
    strata.keyboard.type_text(command)
    strata.keyboard.press("Return")


def test_the_terminal_runs_in_the_browsed_directory_and_survives_hiding(strata):
    directory = strata.fixture.root
    reported = directory / "cwd.txt"

    strata.keyboard.press("F4")
    _run(strata, f"pwd > {reported}")
    strata.wait(lambda: reported.is_file(), "the shell to report its directory")
    assert reported.read_text().strip() == str(directory)

    # $$ is the shell's own pid, so an unchanged value means the same session.
    before = directory / "pid-before.txt"
    after = directory / "pid-after.txt"
    _run(strata, f"echo $$ > {before}")
    strata.wait(lambda: before.is_file(), "the shell to report its pid")

    strata.keyboard.press("F4")
    strata.keyboard.press("F4")

    _run(strata, f"echo $$ > {after}")
    strata.wait(lambda: after.is_file(), "the reopened shell to report its pid")
    assert after.read_text() == before.read_text(), (
        "hiding and showing the panel must keep the same shell"
    )


def test_a_location_without_a_local_path_starts_no_shell(strata):
    reported = strata.fixture.root / "trash-cwd.txt"

    strata.pointer.click(strata.sidebar_button("Trash"))
    strata.wait_for_directory("Trash")

    strata.keyboard.press("F4")
    _run(strata, f"pwd > {reported}")
    time.sleep(QUIET_PERIOD)

    # VTE inherits Strata's own directory when given none, so a shell here
    # would silently report an unrelated local path.
    assert not reported.exists(), (
        "Trash has no local path and must not get a shell at Strata's own cwd"
    )
