# SPDX-License-Identifier: MIT

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
from harness.environment import process_environment
from harness.fixtures import FixtureTree
from harness.process import ManagedProcess, terminate
from harness.tree import Bounds, Node
from tests.e2e.scenarios.test_marquee_scrolling import (
    _entry_bounds,
    _entry_name,
    _visible_entries,
)


VISIBLE = ("showing", "visible")


def _accessible(
    *,
    role: str = "panel",
    name: str = "",
    description: str = "",
    states: tuple[str, ...] = VISIBLE,
    extents: tuple[int, int, int, int] = (0, 0, 10, 10),
    children: list | None = None,
):
    kids = children or []
    acc = Mock()
    acc.get_role_name.return_value = role
    acc.get_name.return_value = name
    acc.get_description.return_value = description
    state_set = Mock()
    state_set.get_states.return_value = [Mock(value_nick=state) for state in states]
    acc.get_state_set.return_value = state_set
    box = Mock(x=extents[0], y=extents[1], width=extents[2], height=extents[3])
    acc.get_extents.return_value = box
    acc.get_child_count.return_value = len(kids)
    acc.get_child_at_index.side_effect = lambda index: kids[index]
    return acc


def _opaque_sibling():
    acc = Mock()
    acc.get_role_name.side_effect = AssertionError("later sibling")
    acc.get_name.side_effect = AssertionError("later sibling")
    acc.get_description.side_effect = AssertionError("later sibling")
    acc.get_state_set.side_effect = AssertionError("later sibling")
    acc.get_extents.side_effect = AssertionError("later sibling")
    acc.get_child_count.return_value = 0
    return acc


def _page(window: Node, monkeypatch) -> Strata:
    monkeypatch.setattr(tree, "set_surface_origin_provider", lambda _provider: None)
    return Strata(
        application=Mock(root=window),
        keyboard=Mock(),
        pointer=Mock(),
        fixture=Mock(),
        environment=Mock(),
        display=Mock(),
    )


@pytest.mark.parametrize("reported", ["button", "push button"])
def test_button_role_is_stable_across_atspi_versions(reported):
    node = Node(Mock(get_role_name=lambda: reported))
    assert node.role == "button"


def test_find_returns_the_first_match_without_querying_later_siblings():
    frame = _accessible(role="frame", name="Strata")
    root = Node(_accessible(role="application", children=[frame, _opaque_sibling()]))
    found = root.find(role="frame", name="Strata")
    assert found is not None
    assert found.name == "Strata"


def test_find_all_returns_every_match_in_walk_order():
    first = _accessible(role="button", name="A")
    second = _accessible(role="button", name="B")
    root = Node(_accessible(children=[first, second]))
    assert [node.name for node in root.find_all(role="button")] == ["A", "B"]


def test_find_all_reads_states_once_when_checking_rendered():
    acc = _accessible(role="label", name="Name")
    Node(acc).find_all(role="label")
    assert acc.get_state_set.call_count == 1


def test_find_all_skips_states_when_rendered_is_off_and_no_state_filter():
    acc = _accessible(role="label", name="Name")
    Node(acc).find_all(role="label", rendered=False)
    acc.get_state_set.assert_not_called()
    acc.get_extents.assert_not_called()


def test_containers_skip_extents_on_non_pane_nodes_and_sort_left_to_right(monkeypatch):
    chrome = _accessible(role="panel", description="toolbar")
    chrome.get_state_set.side_effect = AssertionError("chrome states")
    chrome.get_extents.side_effect = AssertionError("chrome extents")
    right = _accessible(
        role="group", name="docs", description="Columns view", extents=(200, 0, 80, 40)
    )
    left = _accessible(
        role="group", name="root", description="Columns view", extents=(10, 0, 80, 40)
    )
    window = Node(_accessible(role="frame", name="Strata", children=[chrome, right, left]))
    assert [pane.name for pane in _page(window, monkeypatch).containers()] == [
        "root",
        "docs",
    ]


def test_entries_walk_each_pane_once_and_sort_visually(monkeypatch):
    unnamed = _accessible(role="list item")
    decoy = _accessible(role="label", name="todo.txt")
    lower = _accessible(role="list item", name="b.txt", extents=(0, 30, 10, 10))
    upper = _accessible(role="list item", name="a.txt", extents=(0, 8, 10, 10))
    cell = _accessible(role="table cell", name="c.txt", extents=(0, 50, 10, 10))
    files = _accessible(
        role="list",
        name="root",
        description="Files",
        children=[unnamed, decoy, lower, upper, cell],
    )
    pane = _accessible(
        role="group", name="root", description="Columns view", children=[files]
    )
    window = Node(_accessible(role="frame", name="Strata", children=[pane]))
    page = _page(window, monkeypatch)
    assert [entry.name for entry in page.entries()] == ["a.txt", "b.txt", "c.txt"]
    assert decoy.get_role_name.call_count == 1


def test_entries_ignore_a_recycled_row_until_its_label_matches(monkeypatch):
    stale = _accessible(
        role="list item",
        name="blocked",
        extents=(0, 30, 10, 10),
        children=[_accessible(role="label", name="archive")],
    )
    ready = _accessible(
        role="list item",
        name="documents",
        extents=(0, 50, 10, 10),
        children=[_accessible(role="label", name="documents")],
    )
    files = _accessible(
        role="list",
        name="root",
        description="Files",
        children=[stale, ready],
    )
    pane = _accessible(
        role="group", name="root", description="Columns view", children=[files]
    )
    window = Node(_accessible(role="frame", name="Strata", children=[pane]))
    assert [entry.name for entry in _page(window, monkeypatch).entries()] == [
        "documents"
    ]


def test_entries_are_absent_while_two_names_share_a_box(monkeypatch):
    first = _accessible(role="list item", name="archive", extents=(0, 8, 10, 10))
    recycled = _accessible(role="list item", name="blocked", extents=(0, 8, 10, 10))
    files = _accessible(
        role="list",
        name="root",
        description="Files",
        children=[first, recycled],
    )
    pane = _accessible(
        role="group", name="root", description="Columns view", children=[files]
    )
    window = Node(_accessible(role="frame", name="Strata", children=[pane]))
    assert _page(window, monkeypatch).entries() == []


def test_directoryless_entry_lookup_does_not_rebuild_the_pane_list(monkeypatch):
    chrome = _accessible(role="panel", description="chrome")
    first_files = _accessible(
        role="list",
        name="root",
        description="Files",
        children=[_accessible(role="list item", name="readme.md")],
    )
    first = _accessible(
        role="group", name="root", description="Columns view", children=[first_files]
    )
    second_files = _accessible(
        role="list",
        name="docs",
        description="Files",
        children=[_accessible(role="list item", name="todo.txt")],
    )
    second = _accessible(
        role="group", name="docs", description="Columns view", extents=(120, 0, 80, 40),
        children=[second_files],
    )
    window = Node(
        _accessible(role="frame", name="Strata", children=[chrome, first, second])
    )
    found = _page(window, monkeypatch)._entry_or_none("todo.txt", None)
    assert found is not None
    assert found.name == "todo.txt"
    assert chrome.get_description.call_count == 1


def test_application_root_reuses_an_alive_frame(monkeypatch):
    frame = Node(_accessible(role="frame", name="Strata"))
    application = Application(
        display=Mock(), environment=Mock(), location=Path("/tmp"), _frame=frame
    )
    monkeypatch.setattr(
        "harness.application.tree.find_application",
        Mock(side_effect=AssertionError("must not refetch")),
    )
    assert application.root is frame


def test_application_root_refetches_when_the_cached_frame_dies(monkeypatch):
    dead = Mock()
    dead.get_role_name.side_effect = RuntimeError("gone")
    fresh = _accessible(role="frame", name="Strata")
    monkeypatch.setattr(
        "harness.application.tree.find_application",
        lambda _name: Node(_accessible(role="application", children=[fresh])),
    )
    application = Application(
        display=Mock(), environment=Mock(), location=Path("/tmp"), _frame=Node(dead)
    )
    assert application.root.name == "Strata"


def test_marquee_uses_rendered_child_bounds_for_virtualized_cells():
    row = Mock(name="row")
    row.name = "565.txt"
    label = Mock(role="label", screen_bounds=lambda: Bounds(233, 96, 153, 36))
    row.walk.return_value = iter([(0, row), (1, label)])
    row.screen_bounds.return_value = Bounds(217, 7, 191, 145)

    assert _entry_bounds(row) == Bounds(233, 96, 153, 36)
    row.find.assert_not_called()


def test_marquee_uses_rendered_child_identity_for_recycled_cells():
    row = Mock(name="row")
    row.name = "092.txt"
    label = Mock(role="label")
    label.name = "575.txt"
    row.walk.return_value = iter([(0, row), (1, label)])

    assert _entry_name(row) == "575.txt"


def test_marquee_falls_back_to_cell_bounds_when_no_label_is_rendered():
    row = Mock(name="row")
    row.walk.return_value = iter([(0, row)])
    row.screen_bounds.return_value = Bounds(217, 7, 191, 145)

    assert _entry_bounds(row) == Bounds(217, 7, 191, 145)
    row.find.assert_not_called()


def test_marquee_progress_short_circuits_before_inspecting_later_recycled_rows():
    label = Mock(role="label", screen_bounds=lambda: Bounds(20, 30, 80, 20))
    label.name = "060.txt"
    row = Mock(role="list item", window_bounds=lambda: Bounds(20, -10, 80, 20))
    row.walk.side_effect = lambda: iter([(0, row), (1, label)])
    later = Mock(role="list item")
    later.walk.side_effect = AssertionError("must not scan later moving rows")
    container = Mock(
        children=[Mock(role="scroll bar"), row, later],
        screen_bounds=lambda: Bounds(10, 50, 200, 100),
        window_bounds=lambda: Bounds(10, 10, 200, 100),
    )
    viewport = Mock(screen_bounds=lambda: Bounds(10, 20, 200, 100))

    assert any(_entry_name(row) >= "060.txt" for row in _visible_entries(container, viewport))
    container.find_all.assert_not_called()
    later.window_bounds.assert_not_called()
    later.walk.assert_not_called()


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
    pane = Mock()
    pane.screen_bounds.return_value = Bounds(10, 20, 300, 600)
    page = Mock(spec=Strata)
    page._pane_or_none.return_value = pane
    page._entry_container_in.return_value = None
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
