// SPDX-License-Identifier: GPL-3.0-or-later

use super::{clamp_active_after_close, numbered_index, reorder_index, tab_title, wrap_index};
use crate::model::Location;

#[test]
fn home_location_uses_the_familiar_tab_title() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };

    assert_eq!(tab_title(&Location::local(home)), "Home");
}

#[test]
fn wrap_index_cycles_through_tabs() {
    assert_eq!(wrap_index(0, 3, 1), 1);
    assert_eq!(wrap_index(2, 3, 1), 0);
    assert_eq!(wrap_index(0, 3, -1), 2);
    assert_eq!(wrap_index(1, 3, -2), 2);
    assert_eq!(wrap_index(0, 0, 1), 0);
}

#[test]
fn numbered_index_maps_one_based_shortcuts() {
    assert_eq!(numbered_index(1, 3), Some(0));
    assert_eq!(numbered_index(3, 3), Some(2));
    assert_eq!(numbered_index(4, 3), None);
    assert_eq!(numbered_index(0, 3), None);
}

#[test]
fn reorder_index_accounts_for_the_removed_source() {
    assert_eq!(reorder_index(0, 2, false), 1);
    assert_eq!(reorder_index(0, 2, true), 2);
    assert_eq!(reorder_index(2, 0, false), 0);
    assert_eq!(reorder_index(2, 0, true), 1);
}

#[test]
fn closing_a_tab_before_active_shifts_active_down() {
    assert_eq!(clamp_active_after_close(2, 0, 2), 1);
    assert_eq!(clamp_active_after_close(0, 0, 1), 0);
}

#[test]
fn closing_the_active_tab_falls_to_the_next_tab() {
    assert_eq!(clamp_active_after_close(1, 1, 2), 1);
    assert_eq!(clamp_active_after_close(2, 2, 2), 1);
    assert_eq!(clamp_active_after_close(0, 0, 1), 0);
}

#[test]
fn closing_after_active_leaves_active_untouched() {
    assert_eq!(clamp_active_after_close(0, 2, 2), 0);
}
