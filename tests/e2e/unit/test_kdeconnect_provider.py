# SPDX-License-Identifier: MIT
"""Keep the optional provider's unittest suite in the pinned E2E CI selection."""

from pathlib import Path
import subprocess


PROVIDER = Path(__file__).resolve().parents[3] / "contrib/providers/kdeconnect"


def test_kdeconnect_protocol_dbus_and_installation():
    result = subprocess.run(
        ["/usr/bin/python3", "-m", "unittest", "discover", "-s", str(PROVIDER), "-v"],
        capture_output=True, text=True, timeout=90,
        env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"},
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "Ran 0 tests" not in result.stderr
    assert "\nOK\n" in result.stderr
