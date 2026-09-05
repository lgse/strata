// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn pointer_inside_the_dead_zone_does_not_scroll() {
    assert_eq!(autoscroll_step(0.0), 0.0);
    assert_eq!(autoscroll_step(DEAD_ZONE), 0.0);
    assert_eq!(autoscroll_step(-DEAD_ZONE), 0.0);
}

#[test]
fn scroll_speed_follows_the_pointer_direction_and_distance() {
    let near = autoscroll_step(DEAD_ZONE + 40.0);
    let far = autoscroll_step(DEAD_ZONE + 160.0);
    assert!(near > 0.0);
    assert!(far > near);
    assert_eq!(autoscroll_step(-(DEAD_ZONE + 40.0)), -near);
}

#[test]
fn scroll_speed_is_capped_beyond_full_deflection() {
    assert_eq!(
        autoscroll_step(DEAD_ZONE + FULL_SPEED_DISTANCE * 4.0),
        MAX_STEP
    );
}

#[test]
fn a_page_keeps_one_row_of_overlap() {
    assert_eq!(rows_per_page(300.0, 30.0), 9);
}

#[test]
fn a_page_always_moves_at_least_one_row() {
    assert_eq!(rows_per_page(30.0, 30.0), 1);
    assert_eq!(rows_per_page(10.0, 30.0), 1);
    assert_eq!(rows_per_page(300.0, 0.0), 1);
}
