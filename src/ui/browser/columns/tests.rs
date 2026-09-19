// SPDX-License-Identifier: MIT

mod animation;
mod scroll;
mod search;
mod spinner;

use super::drag_scroll::{listing_band_direction, scroll_speed};
use super::*;
use crate::model::Location;
use crate::ui::browser_modes::{ClickActivation, ClickCount};
use std::time::Duration;

#[test]
fn drag_scroll_stays_inside_the_listing_band() {
    assert_eq!(listing_band_direction(10.0, 40.0, 400.0), 0.0);
    assert_eq!(listing_band_direction(450.0, 40.0, 400.0), 0.0);
    assert_eq!(listing_band_direction(240.0, 40.0, 400.0), 0.0);
    assert!(listing_band_direction(80.0, 40.0, 400.0) < 0.0);
    assert!(listing_band_direction(41.0, 40.0, 400.0) < listing_band_direction(80.0, 40.0, 400.0));
    assert!(listing_band_direction(430.0, 40.0, 400.0) > 0.0);
}

#[test]
fn drag_scroll_speed_is_smooth_monotonic_and_continuous() {
    let max = 500.0;
    assert_eq!(scroll_speed(0.0, Duration::ZERO, max), 0.0);
    assert_eq!(scroll_speed(0.0, Duration::from_millis(500), max), 0.0);

    let crawl = scroll_speed(0.05, Duration::ZERO, max);
    let moderate = scroll_speed(0.4, Duration::ZERO, max);
    let full = scroll_speed(1.0, Duration::ZERO, max);
    assert!(crawl > 0.0 && crawl < 5.0);
    assert!(crawl < moderate && moderate < full);

    let dwell_start = scroll_speed(0.8, Duration::ZERO, max);
    let dwell_mid = scroll_speed(0.8, Duration::from_millis(150), max);
    let dwell_full = scroll_speed(0.8, Duration::from_millis(400), max);
    assert!(dwell_start < dwell_mid && dwell_mid < dwell_full);
    assert!((dwell_full - (max * 0.8 * 0.8)).abs() < 1e-3);

    let neg = scroll_speed(-0.5, Duration::from_millis(200), max);
    let pos = scroll_speed(0.5, Duration::from_millis(200), max);
    assert_eq!(neg, -pos);
}

#[test]
fn pointer_preview_handler_ignores_double_click_activation() {
    assert!(should_preview_pointer_press(1, false, false, false));
    assert!(!should_preview_pointer_press(2, false, false, false));
    assert!(!should_preview_pointer_press(1, true, false, false));
    assert!(!should_preview_pointer_press(1, false, true, false));
    assert!(!should_preview_pointer_press(1, false, false, true));
}

#[test]
fn pointer_activation_respects_entry_type_click_count_and_modifiers() {
    let activation = ClickActivation {
        files: ClickCount::Two,
        folders: ClickCount::One,
    };

    assert!(should_activate_single_click(
        1, true, activation, false, false, false
    ));
    assert!(!should_activate_single_click(
        1, false, activation, false, false, false
    ));
    assert!(!should_activate_single_click(
        2, true, activation, false, false, false
    ));
    assert!(!should_activate_single_click(
        1, true, activation, true, false, false
    ));
    assert!(!should_activate_single_click(
        1, true, activation, false, true, false
    ));
    assert!(!should_activate_single_click(
        1, true, activation, false, false, true
    ));
}

#[test]
fn deferred_pointer_activation_requires_an_unchanged_item_without_drag_motion() {
    let location = Location::local("/fixture/folder");
    let mut pending = PendingPointerActivation {
        position: 3,
        location: location.clone(),
        press: (10.0, 20.0),
        moved: false,
        kind: PendingActivationKind::Standard { preview: false },
    };

    pending.update(18.0, 12.0, 8);
    assert!(pending.can_activate(&location));
    assert!(!pending.can_activate(&Location::local("/fixture/replacement")));

    pending.update(19.0, 20.0, 8);
    assert!(!pending.can_activate(&location));
}

#[test]
fn deferred_pointer_activation_remembers_prior_drag_motion() {
    let location = Location::local("/fixture/folder");
    let mut pending = PendingPointerActivation {
        position: 3,
        location: location.clone(),
        press: (10.0, 20.0),
        moved: false,
        kind: PendingActivationKind::Standard { preview: true },
    };

    pending.update(10.0, 29.0, 8);
    pending.update(10.0, 20.0, 8);

    assert!(!pending.can_activate(&location));
}

#[test]
fn pressing_an_item_in_a_multi_selection_preserves_the_drag_group() {
    assert!(should_preserve_drag_selection(true, 2));
    assert!(should_preserve_drag_selection(true, 8));
    assert!(!should_preserve_drag_selection(true, 1));
    assert!(!should_preserve_drag_selection(false, 4));
}

#[test]
fn reveal_target_scrolls_only_enough_to_show_the_new_column() {
    assert_eq!(
        ColumnSpan {
            left: 900.0,
            right: 1200.0,
            total: 1200.0
        }
        .reveal_target(0.0, 900.0, 0.0, 1200.0),
        300.0
    );
}

#[test]
fn reveal_target_moves_only_clipped_columns_with_best_effort_peeks() {
    let column = ColumnSpan {
        left: 900.0,
        right: 1200.0,
        total: 1500.0,
    };
    assert_eq!(column.reveal_target(300.0, 900.0, 0.0, 1500.0), 300.0);
    assert_eq!(column.reveal_target(250.0, 900.0, 0.0, 1500.0), 348.0);
}

#[test]
fn peeks_never_clip_a_fitting_column_or_treat_trailing_space_as_a_neighbor() {
    let middle = ColumnSpan {
        left: 300.0,
        right: 600.0,
        total: 900.0,
    };
    for page in [300.0, 320.0, 348.0, 396.0] {
        let target = middle.reveal_target(600.0, page, 0.0, 900.0);
        assert!(target <= middle.left && target + page >= middle.right);
    }
    let leaf = ColumnSpan {
        left: 600.0,
        right: 900.0,
        total: 900.0,
    };
    assert_eq!(leaf.reveal_target(600.0, 300.0, 0.0, 1500.0), 600.0);
    assert_eq!(
        ColumnSpan {
            left: 0.0,
            right: 300.0,
            total: 900.0
        }
        .reveal_target(300.0, 396.0, 0.0, 900.0),
        0.0
    );
}

#[test]
fn reveal_target_can_scroll_back_to_an_earlier_column() {
    assert_eq!(
        ColumnSpan {
            left: 300.0,
            right: 600.0,
            total: 1500.0
        }
        .reveal_target(600.0, 900.0, 0.0, 1500.0),
        252.0
    );
}
