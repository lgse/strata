// SPDX-License-Identifier: MIT

use super::{ROLES, at_spi, at_spi_toggled};

#[test]
fn runes_roles_read_as_the_at_spi_names_the_scenarios_use() {
    // What the Columns view and the header are made of, as V0's name probe saw them.
    for (rune, expected) in [
        ("window", "frame"),
        ("scroll-view", "scroll pane"),
        ("list-box", "list"),
        ("list-box-option", "list item"),
        ("label", "label"),
        ("button", "button"),
        // The roles the parity set's `role` targets name beside those.
        ("switch", "toggle button"),
        ("text-input", "text"),
        ("document", "text"),
        ("tooltip", "label"),
        ("menu-item", "menu item"),
        ("menu-item-radio", "radio menu item"),
        ("tab-panel", "panel"),
        ("toolbar", "tool bar"),
        ("tree", "tree"),
        ("tree-grid", "tree table"),
        ("tree-item", "tree item"),
    ] {
        assert_eq!(at_spi(rune, true), expected, "{rune}, named");
        assert_eq!(at_spi(rune, false), expected, "{rune}, unnamed");
    }
}

#[test]
fn a_button_with_a_pressed_state_is_a_toggle_button() {
    for (rune, toggled, expected) in [
        ("button", true, "toggle button"),
        ("button", false, "button"),
        ("default-button", true, "toggle button"),
        ("default-button", false, "button"),
        ("check-box", true, "check box"),
        ("menu-item-check-box", true, "check menu item"),
    ] {
        assert_eq!(at_spi_toggled(rune, true, toggled), expected, "{rune}");
    }
}

#[test]
fn only_a_named_group_is_a_grouping() {
    assert_eq!(at_spi("group", true), "grouping");
    assert_eq!(at_spi("group", false), "panel");
    assert_eq!(at_spi("a-role-rune-adds-later", true), "filler");
}

#[test]
fn each_rune_role_is_listed_once() {
    let mut names: Vec<&str> = ROLES.iter().map(|(rune, _)| *rune).collect();
    names.sort_unstable();
    let listed = names.len();
    names.dedup();
    assert_eq!(names.len(), listed);
}
