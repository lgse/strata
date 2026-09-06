# SPDX-License-Identifier: GPL-3.0-or-later
"""Subprocess bookkeeping shared by the display and the application."""

from __future__ import annotations

import os
import signal
import subprocess
from dataclasses import dataclass
from pathlib import Path

TERMINATE_GRACE_SECONDS = 5.0


@dataclass
class ManagedProcess:
    """A child process whose output is captured to a file."""

    name: str
    popen: subprocess.Popen
    log_path: Path

    @classmethod
    def spawn(
        cls,
        name: str,
        command: list[str],
        *,
        log_dir: Path,
        env: dict[str, str] | None = None,
        cwd: Path | None = None,
        **kwargs,
    ) -> "ManagedProcess":
        log_dir.mkdir(parents=True, exist_ok=True)
        log_path = log_dir / f"{name}.log"
        handle = log_path.open("wb")
        try:
            popen = subprocess.Popen(
                command,
                stdin=subprocess.DEVNULL,
                stdout=handle,
                stderr=subprocess.STDOUT,
                env=env,
                cwd=str(cwd) if cwd else None,
                start_new_session=True,
                **kwargs,
            )
        finally:
            handle.close()
        return cls(name=name, popen=popen, log_path=log_path)

    def exited(self) -> bool:
        return self.popen.poll() is not None

    def returncode(self) -> int | None:
        return self.popen.poll()

    def read_log(self) -> str:
        try:
            return self.log_path.read_text(errors="replace")
        except OSError:
            return ""

    def tail(self, lines: int = 40) -> str:
        return "\n".join(self.read_log().splitlines()[-lines:])


def terminate(popen: subprocess.Popen | None) -> None:
    """Stop a process group, escalating to SIGKILL when it does not exit."""

    if popen is None:
        return
    # Every managed process starts a new session. Its group can outlive the
    # leader (notably the accessibility bus launched by at-spi-bus-launcher).
    group = popen.pid
    _signal(popen, group, signal.SIGTERM)
    try:
        popen.wait(timeout=TERMINATE_GRACE_SECONDS)
    except subprocess.TimeoutExpired:
        pass
    _signal(popen, group, signal.SIGKILL)
    try:
        popen.wait(timeout=TERMINATE_GRACE_SECONDS)
    except subprocess.TimeoutExpired:
        pass


def _signal(popen: subprocess.Popen, group: int | None, number: int) -> None:
    try:
        if group is not None:
            os.killpg(group, number)
        else:
            popen.send_signal(number)
    except (ProcessLookupError, PermissionError):
        pass
