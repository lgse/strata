#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""scripts/qa-fuzz.sh hands the fuzz driver's commands to the runner in the pinned image.
The commands are the runner's own, and its tests hold them; what is the script's is to
refuse a call it can see is wrong before it starts a container."""

from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "qa-fuzz.sh"


class QaFuzzScriptTests(unittest.TestCase):
    def test_a_call_without_a_command_or_with_an_unknown_target_starts_no_container(self):
        calls = (
            [],
            ["--alphabet", "calibration"],
            ["run", "--target", "qt"],
            ["run", "--target"],
            ["run", "--target=wayland"],
        )
        for arguments in calls:
            with self.subTest(arguments=arguments):
                # An engine that does not exist: reaching it would fail another way.
                refused = subprocess.run(
                    [str(SCRIPT), *arguments],
                    capture_output=True,
                    text=True,
                    check=False,
                    env={"PATH": "/usr/bin:/bin", "STRATA_CONTAINER_ENGINE": "no-such-engine"},
                )
                self.assertEqual(refused.returncode, 2)
                self.assertIn("Usage: scripts/qa-fuzz.sh", refused.stderr)
                self.assertNotIn("no-such-engine", refused.stderr)
                self.assertEqual(refused.stdout, "")

    def test_a_call_with_a_command_goes_on_to_the_container_engine(self):
        asked = subprocess.run(
            [str(SCRIPT), "check", "--target", "gtk"],
            capture_output=True,
            text=True,
            check=False,
            env={"PATH": "/usr/bin:/bin", "STRATA_CONTAINER_ENGINE": "no-such-engine"},
        )
        self.assertEqual(asked.returncode, 1)
        self.assertIn("QA scenarios require Docker or Podman", asked.stderr)


if __name__ == "__main__":
    unittest.main()
