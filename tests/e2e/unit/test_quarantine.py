# SPDX-License-Identifier: MIT

from types import SimpleNamespace

import pytest

from harness import quarantine


@pytest.mark.parametrize("run_quarantined", [False, True])
def test_quarantine_selects_only_the_recorded_parameter_case(monkeypatch, run_quarantined):
    known = "tests/e2e/scenarios/example.py::test_menu[list]"
    monkeypatch.setattr(quarantine, "NODE_IDS", frozenset({known}))
    markers = [[], [], []]
    items = [
        SimpleNamespace(nodeid=nodeid, add_marker=marks.append)
        for nodeid, marks in zip(
            [known, known + "@visual-baselines", known.replace("[list]", "[icons]")],
            markers,
        )
    ]
    quarantine.apply_quarantine(items, run_quarantined=run_quarantined)
    assert [len(marks) for marks in markers] == (
        [0, 0, 0] if run_quarantined else [1, 1, 0]
    )
    for marks in markers:
        for marker in marks:
            assert marker.name == "skip"
            assert "issues/1154" in marker.kwargs["reason"]
