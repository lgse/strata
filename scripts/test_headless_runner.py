# SPDX-License-Identifier: GPL-3.0-or-later

import importlib.util
import os
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

SPEC = importlib.util.spec_from_file_location(
    "headless_runner", Path(__file__).with_name("test-headless.py")
)
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class HeadlessRunnerTests(unittest.TestCase):
    def test_cargo_uses_only_the_private_display_and_preferences(self):
        display = Mock(environment={"DISPLAY": ":99", "XDG_RUNTIME_DIR": "/private/runtime"})
        home = Mock(variables=lambda: {"HOME": "/private/home"})
        child = Mock(wait=lambda: 0)
        with (
            patch.dict(os.environ, {"DISPLAY": ":0", "WAYLAND_DISPLAY": "wayland-1", "GTK_MODULES": "private"}),
            patch.object(RUNNER, "HeadlessDisplay", return_value=display),
            patch.object(RUNNER, "TestEnvironment", return_value=home),
            patch.object(RUNNER.subprocess, "Popen", return_value=child) as spawn,
            patch.object(RUNNER, "terminate") as terminate,
            patch.object(RUNNER.sys, "argv", ["test-headless.py"]),
        ):
            self.assertEqual(RUNNER.main(), 0)
        environment = spawn.call_args.kwargs["env"]
        self.assertEqual(environment["DISPLAY"], ":99")
        self.assertEqual(environment["HOME"], "/private/home")
        self.assertNotIn("WAYLAND_DISPLAY", environment)
        self.assertNotIn("GTK_MODULES", environment)
        self.assertEqual(environment["STRATA_REQUIRE_GTK_TESTS"], "1")
        terminate.assert_called_once_with(child)
        display.stop.assert_called_once()
        home.cleanup.assert_called_once()

    def test_startup_failure_never_falls_back_to_the_desktop(self):
        display = Mock(start=Mock(side_effect=RuntimeError("no Xvfb")))
        home = Mock()
        with (
            patch.object(RUNNER, "HeadlessDisplay", return_value=display),
            patch.object(RUNNER, "TestEnvironment", return_value=home),
            patch.object(RUNNER.subprocess, "Popen") as spawn,
        ):
            with self.assertRaisesRegex(RuntimeError, "no Xvfb"):
                RUNNER.main()
        spawn.assert_not_called()
        display.stop.assert_called_once()
        home.cleanup.assert_called_once()
