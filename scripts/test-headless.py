#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run workspace Rust tests headless, never against the developer's desktop."""

import os
import signal
import subprocess
import sys
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]

from headless.environment import TestEnvironment, process_environment  # noqa: E402
from headless.process import terminate  # noqa: E402


def test_tmpdir() -> str | None:
    value = os.environ.get("STRATA_TEST_TMPDIR")
    if value and not os.path.isabs(value):
        raise SystemExit(f"STRATA_TEST_TMPDIR must be an absolute path, not {value!r}")
    return value or None


def main() -> int:
    tmpdir = test_tmpdir()
    home = TestEnvironment()
    child = None
    try:
        runtime_dir = home.root / "runtime"
        runtime_dir.mkdir(mode=0o700)
        environment = {
            **process_environment(),
            **home.variables(),
            "XDG_RUNTIME_DIR": str(runtime_dir),
            "CARGO_HOME": os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
            "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
            "WAYLAND_DISPLAY": "strata-test-no-wayland",
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent/strata-test-no-bus",
            "GIO_USE_VFS": "local",
            "STRATA_REQUIRE_DEVICE_TESTS": "1",
            "NO_AT_BRIDGE": "1",
        }
        for name in ("STRATA_DELETE_BENCH_FILES", "STRATA_DELETE_BENCH_ROOT"):
            if name in os.environ:
                environment[name] = os.environ[name]
        if tmpdir:
            environment["TMPDIR"] = tmpdir
        child = subprocess.Popen(
            [
                "cargo",
                "test",
                "--workspace",
                "--all-targets",
                "--all-features",
                *sys.argv[1:],
            ],
            cwd=REPOSITORY,
            env=environment,
            start_new_session=True,
        )
        return child.wait()
    finally:
        terminate(child)
        home.cleanup()


def interrupted(_number, _frame):
    raise SystemExit(143)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    sys.exit(main())
