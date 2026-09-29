# SPDX-License-Identifier: MIT
"""Parametrisation helpers for the three browser presentations."""

from __future__ import annotations

import pytest

# `Strata.view_mode()` reports these names; the stored preference uses the
# lower-case form.
ALL_MODES = [
    pytest.param(
        "Columns", marks=pytest.mark.preferences(browser_mode="columns"), id="columns"
    ),
    pytest.param(
        "Icons", marks=pytest.mark.preferences(browser_mode="icons"), id="icons"
    ),
    pytest.param(
        "List", marks=pytest.mark.preferences(browser_mode="list"), id="list"
    ),
]
SINGLE_PANE_MODES = [mode for mode in ALL_MODES if mode.id != "columns"]

# Icons and List share dialog, menu, and header chrome. Keep Columns when miller
# panes, restored selection, or child-column targeting can still matter.
COLUMNS_AND_ONE = [mode for mode in ALL_MODES if mode.id != "icons"]

# In the grid the next entry sits to the right; the other views stack rows.
NEXT_ENTRY_KEY = {"Columns": "Down", "Icons": "Right", "List": "Down"}
PREVIOUS_ENTRY_KEY = {"Columns": "Up", "Icons": "Left", "List": "Up"}
