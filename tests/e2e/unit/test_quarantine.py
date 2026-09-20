# SPDX-License-Identifier: MIT

from types import SimpleNamespace

import pytest

from harness import quarantine


@pytest.mark.parametrize("run_quarantined", [False, True])
@pytest.mark.parametrize("whole_function", [False, True])
def test_quarantine_scope_and_opt_in(monkeypatch, run_quarantined, whole_function):
    known = "tests/e2e/scenarios/example.py::test_menu[list]"
    selector = known.split("[", 1)[0] if whole_function else known
    monkeypatch.setattr(quarantine, "NODE_IDS", frozenset({selector}))
    markers = [[], [], [], []]
    items = [
        SimpleNamespace(nodeid=nodeid, add_marker=marks.append)
        for nodeid, marks in zip(
            [
                known,
                known + "@visual-baselines",
                known.replace("[list]", "[icons]"),
                known.replace("test_menu", "test_menu_unrelated"),
            ],
            markers,
        )
    ]
    quarantine.apply_quarantine(items, run_quarantined=run_quarantined)
    assert [len(marks) for marks in markers] == (
        [0, 0, 0, 0] if run_quarantined else [1, 1, int(whole_function), 0]
    )
    for marks in markers:
        for marker in marks:
            assert marker.name == "skip"
            assert "issues/1154" in marker.kwargs["reason"]
