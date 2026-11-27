# SPDX-License-Identifier: MIT

import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

SPEC = importlib.util.spec_from_file_location(
    "headless_runner", Path(__file__).with_name("test-headless.py")
)
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class HeadlessRunnerTests(unittest.TestCase):
    def test_check_script_disables_both_desktop_backends(self):
        with tempfile.TemporaryDirectory() as directory:
            tools = Path(directory)
            cargo = tools / "cargo"
            # Tests run headless with no X display, no Wayland socket, and an unconnectable Wayland display.
            cargo.write_text(
                '#!/bin/sh\n'
                '[ "$1" = test ] || exit 0\n'
                '[ -z "${DISPLAY+x}" ] || exit 1\n'
                '[ "$WAYLAND_DISPLAY" = strata-test-no-wayland ] && [ -z "${WAYLAND_SOCKET+x}" ] || exit 1\n'
            )
            cargo.chmod(0o755)
            for name in ("cargo-deny", "typos"):
                tool = tools / name
                tool.write_text('#!/bin/sh\nexit 0\n')
                tool.chmod(0o755)
            result = subprocess.run(
                ["bash", str(Path(__file__).with_name("check.sh"))],
                cwd=directory,
                env={**os.environ, "PATH": f"{directory}:/usr/bin:/bin", "DISPLAY": ":0",
                     "WAYLAND_DISPLAY": "wayland-1", "WAYLAND_SOCKET": "7", "GDK_BACKEND": "wayland"},
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def run_runner(self, inherited):
        with tempfile.TemporaryDirectory() as home_root:
            home = Mock(root=Path(home_root), variables=lambda: {"HOME": "/private/home"})
            child = Mock(wait=lambda: 0)
            with (
                patch.dict(os.environ, {"PATH": "/usr/bin", **inherited}, clear=True),
                patch.object(RUNNER, "TestEnvironment", return_value=home),
                patch.object(RUNNER.subprocess, "Popen", return_value=child) as spawn,
                patch.object(RUNNER, "terminate") as terminate,
                patch.object(RUNNER.sys, "argv", ["test-headless.py"]),
            ):
                self.assertEqual(RUNNER.main(), 0)
            terminate.assert_called_once_with(child)
            home.cleanup.assert_called_once()
            return spawn.call_args.kwargs["env"]

    def test_cargo_runs_headless_with_private_preferences_and_no_desktop_endpoints(self):
        environment = self.run_runner({
            "DISPLAY": ":0", "WAYLAND_DISPLAY": "wayland-1",
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/desktop/bus", "GTK_MODULES": "private",
            "TMPDIR": "/desktop/tmp", "STRATA_TEST_TMPDIR": "",
        })
        self.assertNotIn("DISPLAY", environment)
        self.assertEqual(environment["HOME"], "/private/home")
        self.assertEqual(environment["WAYLAND_DISPLAY"], "strata-test-no-wayland")
        self.assertEqual(environment["DBUS_SESSION_BUS_ADDRESS"], "unix:path=/nonexistent/strata-test-no-bus")
        self.assertEqual(environment["GIO_USE_VFS"], "local")
        self.assertNotIn("GTK_MODULES", environment)
        self.assertNotIn("GTK_A11Y", environment)
        self.assertNotIn("TMPDIR", environment)
        self.assertEqual(environment["NO_AT_BRIDGE"], "1")

    def test_test_tmpdir_moves_only_the_fixtures_tmpdir(self):
        environment = self.run_runner({"TMPDIR": "/desktop/tmp", "STRATA_TEST_TMPDIR": "/mnt/lab/tmp"})
        self.assertEqual(environment["TMPDIR"], "/mnt/lab/tmp")
        self.assertEqual(environment["HOME"], "/private/home")

    def test_relative_test_tmpdir_is_refused_before_anything_starts(self):
        with (
            patch.dict(os.environ, {"STRATA_TEST_TMPDIR": "lab/tmp"}),
            patch.object(RUNNER, "TestEnvironment") as home,
            patch.object(RUNNER.subprocess, "Popen") as spawn,
        ):
            with self.assertRaisesRegex(SystemExit, "STRATA_TEST_TMPDIR must be an absolute path"):
                RUNNER.main()
        home.assert_not_called()
        spawn.assert_not_called()
