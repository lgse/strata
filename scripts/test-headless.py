#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run Rust GUI tests on a private display, never the developer's desktop."""

import os
import signal
import subprocess
import sys
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPOSITORY / "tests/e2e"))

from harness.display import HeadlessDisplay  # noqa: E402
from harness.environment import TestEnvironment, process_environment  # noqa: E402
from harness.process import terminate  # noqa: E402


def main() -> int:
    display = HeadlessDisplay()
    home = TestEnvironment()
    child = None
    try:
        display.start()
        environment = {
            **process_environment(),
            **home.variables(),
            **display.environment,
            "CARGO_HOME": os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
            "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
            "STRATA_REQUIRE_GTK_TESTS": "1",
            "GTK_A11Y": "none",
            "NO_AT_BRIDGE": "1",
        }
        child = subprocess.Popen(
            ["cargo", "test", "--all-targets", "--all-features", *sys.argv[1:]],
            cwd=REPOSITORY,
            env=environment,
            start_new_session=True,
        )
        return child.wait()
    finally:
        terminate(child)
        display.stop()
        home.cleanup()


def interrupted(_number, _frame):
    raise SystemExit(143)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    sys.exit(main())
