# SPDX-License-Identifier: GPL-3.0-or-later
"""Launching and stopping the real Strata binary."""

from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

from . import tree
from .display import HeadlessDisplay
from .environment import TestEnvironment, process_environment
from .process import ManagedProcess, terminate

APPLICATION_NAME = "strata"
# GtkListView reports "list"; GtkGridView reports "layered pane".
ENTRY_CONTAINER_ROLES = ("list", "layered pane", "table")
WINDOW_TITLE = "Strata"
LAUNCH_TIMEOUT = 60.0


def binary_path() -> Path:
    """The debug binary the suite drives."""

    override = os.environ.get("STRATA_BINARY")
    if override:
        path = Path(override)
    else:
        path = repository_root() / "target" / "debug" / "strata"
    if not path.is_file():
        raise FileNotFoundError(
            f"{path} does not exist. Build it with `cargo build` or set "
            "STRATA_BINARY."
        )
    return path.resolve()


def _entry_container(frame: "tree.Node") -> "tree.Node | None":
    for role in ENTRY_CONTAINER_ROLES:
        found = frame.find(role=role)
        if found is not None:
            return found
    return None


def repository_root() -> Path:
    return Path(__file__).resolve().parents[3]


def build_binary() -> Path:
    """Build the debug binary unless one was supplied."""

    if os.environ.get("STRATA_BINARY"):
        return binary_path()
    subprocess.run(
        ["cargo", "build", "--bin", "strata"],
        cwd=repository_root(),
        check=True,
    )
    return binary_path()


@dataclass
class Application:
    """One running Strata process and its accessible root."""

    display: HeadlessDisplay
    environment: TestEnvironment
    location: Path
    process: ManagedProcess | None = None

    def start(self) -> "Application":
        variables = process_environment()
        variables.update(self.environment.variables())
        variables.update(self.display.environment)
        self.process = ManagedProcess.spawn(
            "strata",
            [str(binary_path()), str(self.location)],
            log_dir=self.environment.root,
            env=variables,
            cwd=self.location,
        )
        try:
            self._await_window()
        except BaseException:
            self.stop()
            raise
        return self

    def _await_window(self) -> None:
        tree.connect()

        def window() -> tree.Node | None:
            if self.process is not None and self.process.exited():
                raise AssertionError(
                    "strata exited during startup with "
                    f"{self.process.returncode()}\n{self.process.tail()}"
                )
            application = tree.find_application(APPLICATION_NAME)
            if application is None:
                return None
            frame = application.find(role="frame", name=WINDOW_TITLE)
            if frame is None or not frame.is_rendered():
                return None
            # The browser only becomes drivable once a pane has entries.
            return frame if _entry_container(frame) else None

        tree.wait_until(
            window,
            message="the Strata window to be ready",
            timeout=LAUNCH_TIMEOUT,
            on_timeout=self.diagnostics,
        )

    @property
    def root(self) -> tree.Node:
        application = tree.find_application(APPLICATION_NAME)
        if application is None:
            raise AssertionError("the Strata application is not on the a11y bus")
        frame = application.find(role="frame", name=WINDOW_TITLE)
        if frame is None:
            raise AssertionError("the Strata window is gone")
        return frame

    @property
    def application_node(self) -> tree.Node:
        application = tree.find_application(APPLICATION_NAME)
        if application is None:
            raise AssertionError("the Strata application is not on the a11y bus")
        return application

    def log(self) -> str:
        if self.process:
            return self.process.read_log()
        path = self.environment.root / "strata.log"
        return path.read_text(errors="replace") if path.exists() else ""

    def diagnostics(self) -> str:
        application = tree.find_application(APPLICATION_NAME)
        if application is None:
            return "(the Strata application is not on the accessibility bus)"
        return application.dump()

    def stop(self) -> None:
        if self.process is not None:
            terminate(self.process.popen)
            self.process = None
