# SPDX-License-Identifier: MIT
"""Open with AI agent, across the real process boundary."""

from __future__ import annotations

import os
import time
from pathlib import Path

import pytest

# A fixed location so a preferences marker can name it before Strata starts.
# Every worker writes the same two scripts, so collisions are harmless.
AGENT_DIR = Path("/tmp/strata-e2e-agent")
PROBE = AGENT_DIR / "probe"
BROKEN = AGENT_DIR / "broken"
FINISHING = AGENT_DIR / "finishing"

# The probe records where it ran and what it was given, then stays alive so the
# session it owns is still running when the assertions look at it.
PROBE_SCRIPT = """#!/bin/sh
{
  pwd
  for argument in "$@"; do printf '%s\\n' "$argument"; done
} > agent-report.txt
exec sh
"""
BROKEN_SCRIPT = "#!/definitely/not/a/real/interpreter\necho impossible\n"
FINISHING_SCRIPT = """#!/bin/sh
printf 'AGENT_STARTED\n'
touch agent-started.txt
while [ ! -e agent-finish.txt ]; do
  sleep 0.01
done
printf 'AGENT_FINISHED\n'
touch agent-finished.txt
exit 7
"""

# Long enough for a launch that should not happen to have happened.
QUIET_PERIOD = 3.0


def _install(path: Path, script: str) -> None:
    path.write_text(script)
    path.chmod(0o755)


@pytest.fixture(scope="module", autouse=True)
def agent_scripts():
    AGENT_DIR.mkdir(parents=True, exist_ok=True)
    _install(PROBE, PROBE_SCRIPT)
    _install(BROKEN, BROKEN_SCRIPT)
    _install(FINISHING, FINISHING_SCRIPT)


def _panel_is_open(strata) -> bool:
    return strata.window.find(role="label", name="Terminal") is not None


def _report(directory: Path) -> list[str]:
    return (directory / "agent-report.txt").read_text().splitlines()


def _terminal_primary_selection(strata) -> str:
    from gi.repository import Gdk, GLib

    display = Gdk.Display.open(strata.display.display)
    assert display is not None
    result = []
    try:
        display.get_primary_clipboard().read_text_async(
            None,
            lambda clipboard, response: result.append(
                clipboard.read_text_finish(response) or ""
            ),
        )

        def received():
            context = GLib.MainContext.default()
            while context.pending():
                context.iteration(False)
            return bool(result)

        strata.wait(received, "terminal output on the primary selection")
        return result[0]
    finally:
        display.close()


def _copy_terminal_output(strata) -> str:
    close = strata.window.find(
        role="button", name="End the terminal session (F4 hides it instead)"
    )
    assert close is not None
    header = close.parent
    assert header is not None and header.parent is not None
    terminal = header.parent.children[-1]
    bounds = terminal.screen_bounds()
    strata.pointer.drag_points(
        (bounds.x + bounds.width - 4, bounds.y + bounds.height - 4),
        (bounds.x + 4, bounds.y + 4),
    )
    strata.keyboard.press("ctrl+shift+c")
    return _terminal_primary_selection(strata)


def _run_agent(strata, *, on: str | None = None) -> None:
    if on is None:
        strata.pointer.right_click(strata.pane())
        strata.wait(lambda: strata.context_menu() is not None, "the background menu")
    else:
        strata.open_context_menu(on)
    strata.choose_menu_item("Open with AI agent")


@pytest.mark.preferences(agent_command=str(PROBE))
def test_the_agent_runs_in_the_browsed_folder_with_no_paths(strata):
    _run_agent(strata)

    report = strata.wait(
        lambda: _report(strata.fixture.root) if
        (strata.fixture.root / "agent-report.txt").is_file() else None,
        "the agent to record its launch",
    )
    assert report == [str(strata.fixture.root)], "the background action passes no paths"


@pytest.mark.preferences(agent_command=str(PROBE))
def test_selected_paths_arrive_as_separate_arguments(strata):
    strata.select_entry("readme.md")
    strata.click_entry_with("todo.txt", ["ctrl"])
    _run_agent(strata, on="todo.txt")

    report = strata.wait(
        lambda: _report(strata.fixture.root) if
        (strata.fixture.root / "agent-report.txt").is_file() else None,
        "the agent to record its launch",
    )
    assert report[0] == str(strata.fixture.root)
    assert sorted(report[1:]) == sorted(
        [str(strata.fixture.path("readme.md")), str(strata.fixture.path("todo.txt"))]
    ), "each selected path must arrive as its own argument"


@pytest.mark.preferences(agent_command="./relative-agent")
def test_relative_agent_runs_from_the_requested_folder(strata):
    root = strata.fixture.root
    relative_agent = root / "relative-agent"
    relative_agent.write_text("#!/bin/sh\npwd > relative-agent-report.txt\nexec sh\n")
    relative_agent.chmod(0o755)

    _run_agent(strata)

    report = strata.wait(
        lambda: (root / "relative-agent-report.txt").read_text().splitlines()
        if (root / "relative-agent-report.txt").is_file()
        else None,
        "the relative agent to run from the browsed folder",
    )
    assert report == [str(root)]


@pytest.mark.preferences(agent_command=str(FINISHING))
def test_completed_agent_output_survives_hiding_and_explicit_discard(strata):
    root = strata.fixture.root
    _run_agent(strata)
    strata.wait(
        lambda: (root / "agent-started.txt").is_file(),
        "the agent to start",
    )

    strata.keyboard.press("F4")
    strata.wait(lambda: not _panel_is_open(strata), "the panel to hide")

    (root / "agent-finish.txt").touch()
    strata.wait(
        lambda: (root / "agent-finished.txt").is_file(),
        "the hidden agent to finish",
    )
    assert not _panel_is_open(strata), "a hidden completed agent must stay hidden"

    strata.keyboard.press("F4")
    strata.wait(lambda: _panel_is_open(strata), "the retained output panel")
    for _ in range(2):
        strata.keyboard.press("F4")
        strata.wait(lambda: not _panel_is_open(strata), "the retained output to hide")
        strata.keyboard.press("F4")
        strata.wait(lambda: _panel_is_open(strata), "the retained output to reappear")
    text = _copy_terminal_output(strata)
    assert "AGENT_FINISHED" in text
    assert "exited with status 7" in text

    close = strata.window.find(
        role="button", name="End the terminal session (F4 hides it instead)"
    )
    assert close is not None
    strata.pointer.click(close)
    strata.wait(lambda: not _panel_is_open(strata), "the retained output to close")

    fresh_shell = root / "fresh-shell.txt"
    strata.keyboard.press("F4")
    strata.keyboard.type_text(f"touch {fresh_shell}")
    strata.keyboard.press("Return")
    strata.wait(lambda: fresh_shell.is_file(), "a fresh shell after discard")


@pytest.mark.preferences(agent_command=str(PROBE))
def test_a_non_utf8_working_directory_is_rejected(strata):
    root = strata.fixture.root
    invalid_bytes = os.fsencode(str(root)) + b"/cwd-\xff"
    os.mkdir(invalid_bytes)

    invalid_entry = strata.wait(
        lambda: next(
            (entry for entry in strata.entries() if "\ufffd" in entry.name),
            None,
        ),
        "the invalid UTF-8 directory",
    )
    strata.select_entry(invalid_entry.name)
    strata.wait(
        lambda: (strata.current_directory() or "").startswith("cwd-\ufffd"),
        "the browser to be working in the invalid UTF-8 directory",
    )

    _run_agent(strata)
    dialog = strata.wait(
        lambda: strata.window.find(role="dialog", name="Unable to start the agent"),
        "the cwd encoding error dialog",
    )
    assert dialog is not None
    assert any(
        "without changing it" in label.name
        for label in dialog.find_all(role="label")
    ), f"the dialog should explain the exact-path failure\n{dialog.dump()}"
    assert not (Path(os.fsdecode(invalid_bytes)) / "agent-report.txt").exists()


@pytest.mark.preferences(agent_command=str(PROBE))
def test_an_existing_session_is_never_replaced(strata):
    marker = strata.fixture.root / "shell-alive.txt"
    strata.keyboard.press("F4")
    strata.keyboard.type_text(f"echo $$ > {marker}")
    strata.keyboard.press("Return")
    strata.wait(lambda: marker.is_file(), "the shell to start")
    shell = marker.read_text().strip()

    _run_agent(strata)

    dialog = strata.wait(
        lambda: strata.window.find(role="dialog", name="Unable to start the agent"),
        "the refusal dialog",
    )
    assert dialog is not None
    time.sleep(QUIET_PERIOD)
    assert not (strata.fixture.root / "agent-report.txt").exists(), (
        "the agent must not start while a session is open"
    )

    # The shell that was already there has to survive being refused.
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="dialog", name="Unable to start the agent") is None,
        "the refusal dialog to close",
    )
    marker.unlink()
    strata.keyboard.press("F4")
    strata.keyboard.press("F4")
    strata.keyboard.type_text(f"echo $$ > {marker}")
    strata.keyboard.press("Return")
    strata.wait(lambda: marker.is_file(), "the same shell to answer")
    assert marker.read_text().strip() == shell, "the open session must be untouched"


@pytest.mark.preferences(agent_command=str(BROKEN))
def test_a_launch_that_fails_late_says_so_in_the_panel(strata):
    _run_agent(strata)

    # The spawn passes validation and fails inside VTE, long after the menu has
    # closed. The panel has to stay open to report it, not vanish.
    strata.wait(lambda: _panel_is_open(strata), "the panel to stay open")
    time.sleep(QUIET_PERIOD)
    assert _panel_is_open(strata), "a failed launch must not hide the panel"

    # Losing the window would mean the failure took Strata with it, and the
    # generic terminal has to remain usable afterwards.
    strata.entry("readme.md")
    marker = strata.fixture.root / "recovered.txt"
    strata.keyboard.press("F4")
    strata.keyboard.press("F4")
    strata.keyboard.type_text(f"echo recovered > {marker}")
    strata.keyboard.press("Return")
    strata.wait(lambda: marker.is_file(), "a shell to start after the failed agent")
