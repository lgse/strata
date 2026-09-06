# SPDX-License-Identifier: GPL-3.0-or-later
"""Deterministic headless display, session bus, and accessibility bus."""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

from .environment import TestEnvironment, process_environment
from .process import ManagedProcess, terminate

# `sockaddr_un.sun_path` holds 107 usable bytes. `at-spi-bus-launcher` binds
# `$XDG_RUNTIME_DIR/at-spi/bus_<display>`; when that path does not fit it falls
# back to advertising the address `unix:abstract=`, and every libatspi client
# then aborts inside a fatal GLib error that Python cannot catch. Keep the
# runtime directory short and verify the advertised address before use.
MAX_RUNTIME_DIR_LENGTH = 60

# Well clear of the session display and of the numbers Xephyr and friends take.
FIRST_DISPLAY_NUMBER = 90
DISPLAY_TRIES = 30
XVFB_START_TIMEOUT = 15.0

BUS_LAUNCHER_CANDIDATES = (
    "/usr/libexec/at-spi-bus-launcher",
    "/usr/lib/at-spi-bus-launcher",
    "/usr/lib/at-spi2-core/at-spi-bus-launcher",
    "/usr/libexec/at-spi2-core/at-spi-bus-launcher",
)
REGISTRY_CANDIDATES = (
    "/usr/libexec/at-spi2-registryd",
    "/usr/lib/at-spi2-registryd",
    "/usr/lib/at-spi2-core/at-spi2-registryd",
    "/usr/libexec/at-spi2-core/at-spi2-registryd",
)

# A session bus with no service directories. Nothing on this bus is
# D-Bus-activatable, which keeps the desktop portal, systemd, and the document
# portal's FUSE mount out of the test session.
SESSION_BUS_CONFIG = """<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={socket}</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"""


class DisplayError(RuntimeError):
    """The headless display or one of its buses could not be started."""


def _locate(candidates: tuple[str, ...], name: str) -> str:
    for candidate in candidates:
        if os.access(candidate, os.X_OK):
            return candidate
    found = shutil.which(name)
    if found:
        return found
    raise DisplayError(
        f"{name} was not found. Install at-spi2-core "
        f"(searched {', '.join(candidates)})."
    )


@dataclass
class HeadlessDisplay:
    """An Xvfb display plus the private buses the accessibility stack needs."""

    width: int = 1440
    height: int = 900
    depth: int = 24
    dpi: int = 96
    startup_timeout: float = 30.0

    runtime_dir: Path = field(init=False)
    display: str = field(init=False)
    session_bus_address: str = field(init=False)
    accessibility_bus_address: str = field(init=False)
    _processes: list[ManagedProcess] = field(init=False, default_factory=list)
    _home: TestEnvironment | None = field(init=False, default=None)

    def start(self) -> None:
        self.runtime_dir = Path(tempfile.mkdtemp(prefix="strata-e2e-", dir="/tmp"))
        self._home = TestEnvironment()
        if len(str(self.runtime_dir)) > MAX_RUNTIME_DIR_LENGTH:
            self.stop()
            raise DisplayError(
                f"runtime directory {self.runtime_dir} is too long; the "
                "accessibility bus socket would exceed the 107-byte limit"
            )
        self.runtime_dir.chmod(0o700)
        try:
            self._start_xvfb()
            self._start_session_bus()
            self._start_accessibility_bus()
            self._start_registry()
        except BaseException as error:
            logs = self.logs()
            self.stop()
            if isinstance(error, Exception):
                raise DisplayError(f"{error}\nSession logs: {logs}") from error
            raise

    @property
    def environment(self) -> dict[str, str]:
        return {
            "DISPLAY": self.display,
            "DBUS_SESSION_BUS_ADDRESS": self.session_bus_address,
            "AT_SPI_BUS_ADDRESS": self.accessibility_bus_address,
            "XDG_RUNTIME_DIR": str(self.runtime_dir),
        }

    def _spawn(self, name: str, command: list[str], **kwargs) -> ManagedProcess:
        if "env" not in kwargs:
            kwargs["env"] = {**process_environment(), **self._home.variables()}
        managed = ManagedProcess.spawn(name, command, log_dir=self.runtime_dir, **kwargs)
        self._processes.append(managed)
        return managed

    def _start_xvfb(self) -> None:
        # `-displayfd` always searches upwards from :0, which on a developer
        # machine means the real session display. Claim a high number instead
        # and move on when the server refuses it.
        for number in range(FIRST_DISPLAY_NUMBER, FIRST_DISPLAY_NUMBER + DISPLAY_TRIES):
            if self._display_is_taken(number):
                continue
            if self._try_display(number):
                self.display = f":{number}"
                return
        raise DisplayError(
            f"no free X display in :{FIRST_DISPLAY_NUMBER}-"
            f":{FIRST_DISPLAY_NUMBER + DISPLAY_TRIES - 1}"
        )

    @staticmethod
    def _display_is_taken(number: int) -> bool:
        return (
            Path(f"/tmp/.X{number}-lock").exists()
            or Path(f"/tmp/.X11-unix/X{number}").exists()
        )

    def _try_display(self, number: int) -> bool:
        server = self._spawn(
            f"xvfb-{number}",
            [
                "Xvfb",
                f":{number}",
                "-screen",
                "0",
                f"{self.width}x{self.height}x{self.depth}",
                "-dpi",
                str(self.dpi),
                "-nolisten",
                "tcp",
                "-noreset",
                "-ac",
            ],
        )
        socket = Path(f"/tmp/.X11-unix/X{number}")
        deadline = time.monotonic() + XVFB_START_TIMEOUT
        while time.monotonic() < deadline:
            if socket.exists():
                return True
            if server.exited():
                self._processes.remove(server)
                return False
            time.sleep(0.05)
        terminate(server.popen)
        self._processes.remove(server)
        return False

    def _start_session_bus(self) -> None:
        socket = self.runtime_dir / "bus"
        config = self.runtime_dir / "session-bus.conf"
        config.write_text(SESSION_BUS_CONFIG.format(socket=socket))
        self.session_bus_address = f"unix:path={socket}"
        self._spawn(
            "dbus-daemon",
            ["dbus-daemon", "--nofork", "--nopidfile", f"--config-file={config}"],
        )
        self._wait_for(socket.exists, "the private session bus did not start")

    def _start_accessibility_bus(self) -> None:
        launcher = _locate(BUS_LAUNCHER_CANDIDATES, "at-spi-bus-launcher")
        self._spawn(
            "at-spi-bus-launcher",
            [launcher, "--launch-immediately"],
            env=self._bus_environment(),
        )
        address = ""

        def advertised() -> bool:
            nonlocal address
            address = self._query_accessibility_address()
            return address.startswith("unix:path=") or address.startswith(
                "unix:abstract="
            ) and len(address) > len("unix:abstract=")

        self._wait_for(advertised, "the accessibility bus did not advertise an address")
        if not address.startswith("unix:path="):
            raise DisplayError(
                f"the accessibility bus advertised {address!r}; the socket path "
                "is most likely too long"
            )
        self.accessibility_bus_address = address

    def _query_accessibility_address(self) -> str:
        reply = subprocess.run(
            [
                "dbus-send",
                "--print-reply=literal",
                "--dest=org.a11y.Bus",
                "/org/a11y/bus",
                "org.a11y.Bus.GetAddress",
            ],
            env=self._bus_environment(),
            capture_output=True,
            text=True,
            timeout=5,
        )
        if reply.returncode != 0:
            return ""
        return reply.stdout.strip()

    def _start_registry(self) -> None:
        # The registry is normally D-Bus-activated. The private session bus has
        # no service directories, so start it directly.
        registry = _locate(REGISTRY_CANDIDATES, "at-spi2-registryd")
        self._spawn(
            "at-spi2-registryd",
            [registry],
            env=self._bus_environment(),
        )

    def _bus_environment(self) -> dict[str, str]:
        environment = {**process_environment(), **self._home.variables()}
        environment.update(
            {
                "DISPLAY": self.display,
                "DBUS_SESSION_BUS_ADDRESS": self.session_bus_address,
                "XDG_RUNTIME_DIR": str(self.runtime_dir),
                "XDG_SESSION_TYPE": "x11",
                "GDK_BACKEND": "x11",
            }
        )
        address = getattr(self, "accessibility_bus_address", None)
        if address:
            environment["AT_SPI_BUS_ADDRESS"] = address
        return environment

    def _wait_for(self, condition, message: str) -> None:
        deadline = time.monotonic() + self.startup_timeout
        while time.monotonic() < deadline:
            for process in self._processes:
                if process.exited():
                    raise DisplayError(
                        f"{message}: {process.name} exited with "
                        f"{process.returncode()}\n{process.tail()}"
                    )
            if condition():
                return
            time.sleep(0.1)
        raise DisplayError(f"{message} within {self.startup_timeout:.0f}s")

    def logs(self) -> dict[str, str]:
        return {process.name: process.tail() for process in self._processes}

    def stop(self) -> None:
        for process in reversed(self._processes):
            terminate(process.popen)
        self._processes.clear()
        runtime_dir = getattr(self, "runtime_dir", None)
        if runtime_dir is not None:
            shutil.rmtree(runtime_dir, ignore_errors=True)
        if self._home is not None:
            self._home.cleanup()
            self._home = None
