# SPDX-License-Identifier: GPL-3.0-or-later
"""Column background clicks preserve the open path and selection."""

import pytest


@pytest.mark.preferences(browser_mode="columns")
@pytest.mark.parametrize("surface", ["content", "header"])
def test_column_background_click_focuses_parent(strata, surface):
    root = strata.fixture.root.name
    strata.open_directory("documents")
    selected = strata.selected_names(directory=root)
    if surface == "content":
        strata.pointer.click(strata.pane(root), at=strata.background_point(root))
    else:
        heading = strata.window.find(
            role="label", name=root, description=str(strata.fixture.root)
        )
        assert heading is not None
        strata.pointer.click(heading)
    strata.wait_for_directory(root)
    assert strata.selected_names(directory=root) == selected
    assert "documents" in strata.pane_names()
