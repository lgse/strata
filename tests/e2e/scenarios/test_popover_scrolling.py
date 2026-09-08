# SPDX-License-Identifier: GPL-3.0-or-later
"""Outside wheel ticks dismiss browser panels and only scroll the pointed listing."""

import pytest

from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES

PANELS = [
    pytest.param(mode.values[0], panel, marks=mode.marks, id=f"{panel}-{mode.id}")
    for mode in ALL_MODES
    for panel in ["sort", "appearance", "thumbnail"]
    if (panel != "sort" or mode.id != "list")
    and (panel != "thumbnail" or mode.id == "icons")
]


@pytest.fixture
def fixture_tree():
    files = {f"{index:03}.txt": f"{index}\n" for index in range(200)}
    fixture = FixtureTree.create({"nested": files, **files})
    try:
        yield fixture
    finally:
        fixture.cleanup()


def viewport(strata, directory=None):
    for node in strata.entry_container(directory).ancestors():
        if node.role == "scroll pane":
            return node.screen_bounds()
    raise AssertionError("listing has no scroll viewport")


def open_panel(strata, panel):
    if panel == "sort":
        strata.pointer.click(strata.header_button("Choose sort field"))
        label = "Folders first"
    elif panel == "appearance":
        strata.open_appearance_menu()
        label = "Compact"
    else:
        strata.pointer.click(strata.header_button("Thumbnail size"))
        label = "Small"
    role = "label" if panel == "thumbnail" else "button"
    return strata.wait(
        lambda: strata.window.find(role=role, name=label),
        f"the {panel} panel to open",
    )


@pytest.mark.parametrize("mode,panel", PANELS)
@pytest.mark.parametrize("target", ["listing", "sidebar", "inside"])
def test_panel_wheel_routing(strata, mode, panel, target):
    row = strata.entry("005.txt")
    strata.settle(row)
    before = row.screen_bounds()
    bounds = viewport(strata)
    option = open_panel(strata, panel)
    if target == "listing":
        point = (bounds.center[0], bounds.y + bounds.height * 4 // 5)
    elif target == "sidebar":
        point = strata.sidebar_button("Home").screen_bounds().center
    else:
        point = option.screen_bounds().center
    strata.pointer.scroll(at=point, clicks=1)
    if target == "inside":
        assert strata.window.find(role=option.role, name=option.name) is not None
        assert row.screen_bounds() == before
    else:
        strata.wait(
            lambda: strata.window.find(role=option.role, name=option.name) is None,
            "outside wheel to close the panel",
        )
        if target == "listing":
            strata.wait(
                lambda: row.screen_bounds().y < before.y,
                "the same wheel tick to move the listing",
            )
        else:
            assert row.screen_bounds() == before


@pytest.mark.parametrize("panel", ["sort", "appearance"])
@pytest.mark.parametrize("pointed_column", ["parent", "child"])
def test_outside_wheel_only_moves_the_column_under_the_pointer(
    strata, panel, pointed_column
):
    root = strata.fixture.root.name
    strata.open_directory("nested")
    parent_row = strata.entry("005.txt", directory=root)
    child_row = strata.entry("005.txt", directory="nested")
    strata.settle(parent_row)
    strata.settle(child_row)
    parent_before = parent_row.screen_bounds()
    child_before = child_row.screen_bounds()
    option = open_panel(strata, panel)
    bounds = viewport(strata, root if pointed_column == "parent" else "nested")
    strata.pointer.scroll(
        at=(bounds.center[0], bounds.y + bounds.height * 4 // 5), clicks=1
    )
    strata.wait(
        lambda: strata.window.find(role=option.role, name=option.name) is None,
        "outside wheel to close the panel",
    )
    if pointed_column == "parent":
        strata.wait(
            lambda: parent_row.screen_bounds().y < parent_before.y, "parent to scroll"
        )
        assert child_row.screen_bounds() == child_before
    else:
        strata.wait(
            lambda: child_row.screen_bounds().y < child_before.y, "child to scroll"
        )
        assert parent_row.screen_bounds() == parent_before
