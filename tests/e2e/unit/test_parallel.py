# SPDX-License-Identifier: GPL-3.0-or-later

from concurrent.futures import ThreadPoolExecutor
from unittest.mock import Mock

import pytest

from harness.artifacts import ArtifactCollector
from harness.display import HeadlessDisplay


def test_artifacts_are_separated_by_worker(tmp_path, monkeypatch):
    monkeypatch.setenv("STRATA_E2E_ARTIFACTS", str(tmp_path))
    paths = []
    for worker in ("gw0", "gw1"):
        monkeypatch.setenv("PYTEST_XDIST_WORKER", worker)
        paths.append(ArtifactCollector("infrastructure").write("startup-error.txt", worker))
    assert paths[0] != paths[1]
    assert [p.read_text() for p in paths] == ["gw0", "gw1"]


def test_concurrent_displays_have_private_servers_and_buses():
    displays = [HeadlessDisplay() for _ in range(4)]
    try:
        with ThreadPoolExecutor(max_workers=4) as executor:
            list(executor.map(lambda display: display.start(), displays))
        for attribute in ("display", "runtime_dir", "session_bus_address", "accessibility_bus_address"):
            assert len({getattr(display, attribute) for display in displays}) == len(displays)
        assert all(not process.exited() for display in displays for process in display._processes)
    finally:
        for display in displays:
            display.stop()
    assert all(not display.runtime_dir.exists() for display in displays)


def test_display_does_not_accept_another_workers_socket(monkeypatch):
    display = HeadlessDisplay()
    server = Mock(exited=lambda: True)
    monkeypatch.setattr(display, "_spawn", lambda *args, **kwargs: server)
    monkeypatch.setattr("harness.display.Path.exists", lambda _: True)
    display._processes.append(server)
    assert not display._try_display(1999)
    assert not display._processes


def test_baselines_are_grouped_before_xdist_scheduling():
    from conftest import pytest_collection_modifyitems

    baseline, ordinary = Mock(), Mock()
    ordinary.get_closest_marker.return_value = None
    pytest_collection_modifyitems([baseline, ordinary])
    assert baseline.add_marker.call_args.args[0].args == ("visual-baselines",)
    ordinary.add_marker.assert_not_called()


def test_unsafe_scheduler_is_rejected():
    from conftest import pytest_configure

    config = Mock(getoption=lambda name, default: {"numprocesses": 2, "dist": "load"}[name])
    with pytest.raises(pytest.UsageError, match="loadgroup"):
        pytest_configure(config)
