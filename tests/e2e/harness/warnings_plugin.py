# SPDX-License-Identifier: MIT
"""Keep PyGObject diagnostics reportable across pytest-xdist workers."""

from warnings import WarningMessage

import pytest


@pytest.hookimpl(tryfirst=True)
def pytest_warning_recorded(warning_message: WarningMessage) -> None:
    category = warning_message.category
    if (category.__module__, category.__name__) != ("gobject", "Warning"):
        return
    # xdist imports warning classes by module/name, but PyGObject deliberately
    # blocks the legacy gobject module. Normalize only the captured report;
    # warning filters (including errors) have already run at the emission site.
    warning_message.message = Warning(f"gobject.Warning: {warning_message.message}")
    warning_message.category = Warning
