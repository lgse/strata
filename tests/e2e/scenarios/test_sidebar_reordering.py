# SPDX-License-Identifier: MIT

def test_dragging_trash_above_home_reorders_the_sidebar(strata):
    home = strata.sidebar_button("Home")
    trash = strata.sidebar_button("Trash")
    assert [row.name for row in home.parent.children[:2]] == ["Home", "Trash"]

    bounds = home.screen_bounds()
    strata.pointer.drag_points(
        trash.screen_bounds().center,
        (bounds.center[0], bounds.y + max(1, bounds.height // 5)),
        steps=12,
    )

    strata.wait(
        lambda: [row.name for row in strata.sidebar_button("Home").parent.children[:2]]
        == ["Trash", "Home"],
        "the dropped Trash row to precede Home",
    )
