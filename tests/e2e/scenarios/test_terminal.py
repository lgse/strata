# SPDX-License-Identifier: GPL-3.0-or-later

from __future__ import annotations

import pytest

from harness import environment as harness_environment

LAUNCHER = "xdg-terminal-exec"


@pytest.fixture
def test_environment():
    """Empty PATH so launcher availability does not depend on the host."""

    environment = harness_environment.TestEnvironment()
    inherited = environment.variables
    empty_path = environment.root / "empty-path"
    empty_path.mkdir()

    def variables():
        return {**inherited(), "PATH": str(empty_path)}

    environment.variables = variables
    try:
        yield environment
    finally:
        environment.cleanup()


def test_a_missing_terminal_launcher_is_named_in_the_error(strata):
    strata.keyboard.press("ctrl+t")

    dialog = strata.wait_for_dialog()
    assert dialog.name == "Unable to open terminal", (
        f"unexpected dialog {dialog.name!r}"
    )
    assert any(LAUNCHER in node.name for node in dialog.find_all(role="label")), (
        f"the error should name the missing launcher\n{dialog.dump()}"
    )

    strata.pointer.click(strata.dialog_button("Close"))
    strata.wait(lambda: strata.dialog() is None, "the dialog to close")
