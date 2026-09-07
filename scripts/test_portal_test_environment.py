# SPDX-License-Identifier: GPL-3.0-or-later

import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from portal_test_environment import isolated_process_environment


class PortalTestEnvironmentTests(unittest.TestCase):
    def test_isolated_backend_disables_host_accessibility_bus_discovery(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with mock.patch.dict(os.environ, {"GTK_A11Y": "atspi"}):
                environment = isolated_process_environment(root)

        self.assertEqual(environment["GTK_A11Y"], "none")
        self.assertEqual(environment["GDK_DEBUG"], "no-portals")
        self.assertEqual(environment["GIO_USE_VFS"], "local")
        self.assertEqual(environment["XDG_CONFIG_HOME"], str(root / "config"))
        self.assertEqual(environment["XDG_DATA_HOME"], str(root / "data"))
        self.assertEqual(environment["XDG_CACHE_HOME"], str(root / "cache"))
