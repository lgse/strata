# SPDX-License-Identifier: MIT

from pathlib import Path
import subprocess
import sys
from warnings import WarningMessage

import gi
from gi.repository import GObject
import pytest
from xdist.remote import serialize_warning_message
from xdist.workermanage import unserialize_warning_message

from harness.environment import process_environment
from harness.warnings_plugin import pytest_warning_recorded


def test_gobject_warning_transport_preserves_diagnostics():
    message = WarningMessage(
        GObject.Warning("diagnostic details"), GObject.Warning, "source.py", 42,
        line="the warning source",
    )
    pytest_warning_recorded(message)
    received = unserialize_warning_message(serialize_warning_message(message))
    assert received.category is Warning
    assert str(received.message) == "gobject.Warning: diagnostic details"
    assert (received.filename, received.lineno, received.line) == (
        "source.py", 42, "the warning source",
    )


@pytest.mark.parametrize("category", [UserWarning, gi.PyGIWarning, gi.PyGIDeprecationWarning])
def test_importable_warning_categories_are_unchanged(category):
    original = category("diagnostic details")
    message = WarningMessage(original, category, "source.py", 42)
    pytest_warning_recorded(message)
    assert message.message is original
    assert message.category is category


@pytest.mark.parametrize("phase", ["collection", "call", "teardown"])
@pytest.mark.parametrize("policy", ["always", "error"])
def test_real_workers_report_gobject_warnings_without_crashing(tmp_path, phase, policy):
    emission = 'warnings.warn("warning transport regression", GObject.Warning)'
    bodies = {
        "collection": f"{emission}\ndef test_case():\n    pass\n",
        "call": f"def test_case():\n    {emission}\n",
        "teardown": f"def test_case():\n    pass\ndef teardown_function():\n    {emission}\n",
    }
    (tmp_path / "pytest.ini").write_text("[pytest]\n")
    (tmp_path / "test_warning.py").write_text(
        "import warnings\nfrom gi.repository import GObject\n" + bodies[phase]
    )
    result = subprocess.run(
        [sys.executable, "-m", "pytest", "-p", "xdist.plugin",
         "-p", "harness.warnings_plugin", "-n", "2", "--dist=loadgroup",
         "-W", f"{policy}:warning transport regression"],
        cwd=tmp_path,
        env={**process_environment(), "PYTHONPATH": str(Path(__file__).resolve().parents[1]),
             "PYTEST_DISABLE_PLUGIN_AUTOLOAD": "1"},
        capture_output=True, text=True, timeout=30,
    )
    output = result.stdout + result.stderr
    assert "INTERNALERROR" not in output, output
    assert "gobject.Warning: warning transport regression" in output, output
    assert "test_warning.py:" in output, output
    if policy == "error":
        assert result.returncode in (1, 2), output
    else:
        assert result.returncode == 0, output
        assert "1 passed" in output, output
