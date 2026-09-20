# SPDX-License-Identifier: MIT
"""Exact-case quarantine for tracked regressions; opt in with --run-quarantined."""

import json
from pathlib import Path

import pytest

REASON = "Native-menu regression; https://github.com/lgse/strata/issues/1154"
NODE_IDS = frozenset(
    json.loads((Path(__file__).resolve().parents[1] / "quarantined.json").read_text())
)


def apply_quarantine(items, *, run_quarantined: bool) -> None:
    if run_quarantined:
        return
    for item in items:
        if item.nodeid.removesuffix("@visual-baselines") in NODE_IDS:
            item.add_marker(pytest.mark.skip(reason=REASON))
