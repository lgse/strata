# SPDX-License-Identifier: GPL-3.0-or-later

import os
import signal
import sys
from pathlib import Path
from unittest.mock import Mock, call

import pytest
from PIL import Image

from harness import screenshots, tree
from harness.application import Application, binary_path
from harness.browser import Strata
from harness.tree import Bounds, Node
from harness.environment import process_environment
from harness.fixtures import FixtureTree
from harness.process import ManagedProcess, terminate


@pytest.mark.parametrize("reported", ["button", "push button"])
def test_button_role_is_stable_across_atspi_versions(reported):
    node = Node(Mock(get_role_name=lambda: reported))
    assert node.role == "button"


@pytest.mark.parametrize("anchor", [(0, 0), (1074, 6), (500, 200)])
def test_popup_bounds_account_for_native_surface_origins(monkeypatch, anchor):
    frame = Mock(spec=Node)
    frame._bounds.return_value = Bounds(0, 0, 1200, 760)
    surface = Mock(spec=Node)
    surface.window_bounds.return_value = Bounds(*anchor, 258, 475)
    node = Mock(spec=Node)
    node.toplevel.return_value = frame
    node.ancestors.return_value = iter([surface, frame])
    node.window_bounds.return_value = Bounds(anchor[0] + 30, anchor[1] + 60, 200, 28)
    origin = Mock(side_effect=lambda w, h: (500, 200) if (w, h) == (258, 475) else None)
    monkeypatch.setattr(tree, "_surface_origin", origin)
    assert Node.screen_bounds(node) == Bounds(530, 260, 200, 28)
    assert origin.call_args_list == [call(200, 28), call(258, 475)]


def test_empty_pane_context_target_avoids_the_paste_footer():
    page = Mock(spec=Strata)
    page.entry_container.return_value = None
    page.pane.return_value.screen_bounds.return_value = Bounds(10, 20, 300, 600)
    assert Strata.background_point(page, "empty") == (160, 320)


def test_fixed_fixture_refuses_existing_directory(tmp_path):
    root = tmp_path / "fixture"
    root.mkdir()
    sentinel = root / "user-data"
    sentinel.write_text("keep me")
    with pytest.raises(FileExistsError):
        FixtureTree.create_at(root)
    assert sentinel.read_text() == "keep me"


def test_fixture_permissions_do_not_depend_on_umask(tmp_path):
    mask = os.umask(0o077)
    try:
        fixture = FixtureTree.create_at(tmp_path / "fixture", {"folder": {"file": "data"}})
    finally:
        os.umask(mask)
    assert fixture.path("folder").stat().st_mode & 0o777 == 0o755
    assert fixture.path("folder/file").stat().st_mode & 0o777 == 0o644


def test_fixed_fixture_refuses_symlink(tmp_path):
    root = tmp_path / "fixture"
    root.symlink_to(tmp_path / "missing")
    with pytest.raises(FileExistsError):
        FixtureTree.create_at(root)
    assert root.is_symlink()


@pytest.mark.parametrize("channel", range(3))
def test_visual_comparison_checks_each_channel(tmp_path, monkeypatch, channel):
    baselines = tmp_path / "baselines"
    baselines.mkdir()
    monkeypatch.setattr(screenshots, "BASELINE_DIRECTORY", baselines)
    monkeypatch.delenv("STRATA_E2E_UPDATE_BASELINES", raising=False)
    Image.new("RGB", (10, 10)).save(baselines / "test.png")
    color = [0, 0, 0]
    color[channel] = screenshots.CHANNEL_TOLERANCE + 1
    actual = tmp_path / "actual.png"
    Image.new("RGB", (10, 10), tuple(color)).save(actual)
    result = screenshots.compare_to_baseline("test", actual, tmp_path / "artifacts")
    assert not result.matched
    assert result.different_fraction == 1
    assert result.diff.is_file()


def test_visual_comparison_accepts_channel_tolerance(tmp_path, monkeypatch):
    monkeypatch.setattr(screenshots, "BASELINE_DIRECTORY", tmp_path)
    monkeypatch.delenv("STRATA_E2E_UPDATE_BASELINES", raising=False)
    Image.new("RGB", (10, 10)).save(tmp_path / "test.png")
    actual = tmp_path / "actual.png"
    Image.new("RGB", (10, 10), (24, 24, 24)).save(actual)
    assert screenshots.compare_to_baseline("test", actual, tmp_path / "artifacts").matched


def test_session_overrides_do_not_leak(monkeypatch):
    for name in ("GTK_MODULES", "FONTCONFIG_FILE", "DBUS_SESSION_BUS_ADDRESS", "AT_SPI_BUS_ADDRESS", "WAYLAND_DISPLAY", "LD_PRELOAD"):
        monkeypatch.setenv(name, "private-session")
    assert process_environment() == {"PATH": os.environ["PATH"]}


def test_relative_binary_override_survives_the_fixture_working_directory(monkeypatch, tmp_path):
    binary = tmp_path / "strata"
    binary.touch()
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("STRATA_BINARY", "strata")
    assert binary_path() == binary


def test_application_start_failure_stops_process(monkeypatch, tmp_path):
    process = Mock()
    monkeypatch.setattr("harness.application.binary_path", lambda: Path("/test/strata"))
    monkeypatch.setattr(ManagedProcess, "spawn", Mock(return_value=process))
    stop = Mock()
    monkeypatch.setattr("harness.application.terminate", stop)
    monkeypatch.setattr(Application, "_await_window", Mock(side_effect=RuntimeError("startup")))
    application = Application(
        display=Mock(environment={}),
        environment=Mock(root=tmp_path, variables=lambda: {}),
        location=tmp_path,
    )
    with pytest.raises(RuntimeError, match="startup"):
        application.start()
    stop.assert_called_once_with(process.popen)
    assert application.process is None


def test_terminate_signals_group_after_leader_exits(monkeypatch):
    process = Mock(pid=123, poll=lambda: 0)
    killpg = Mock()
    monkeypatch.setattr(os, "killpg", killpg)
    terminate(process)
    assert killpg.call_args_list == [
        ((123, signal.SIGTERM),),
        ((123, signal.SIGKILL),),
    ]


def test_managed_process_is_reaped(tmp_path):
    process = ManagedProcess.spawn(
        "child", [sys.executable, "-c", "import time; time.sleep(60)"], log_dir=tmp_path
    )
    terminate(process.popen)
    assert process.popen.poll() is not None
