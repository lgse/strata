// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn pointer_inside_the_viewport_does_not_scroll() {
    assert_eq!(auto_scroll_step(200.0, 400.0), 0.0);
    assert_eq!(auto_scroll_step(AUTO_SCROLL_MARGIN, 400.0), 0.0);
}

#[test]
fn pointer_near_an_edge_scrolls_towards_it() {
    assert!(auto_scroll_step(4.0, 400.0) < 0.0);
    assert!(auto_scroll_step(396.0, 400.0) > 0.0);
}

#[test]
fn scroll_speed_saturates_beyond_the_viewport() {
    assert_eq!(auto_scroll_step(-500.0, 400.0), -AUTO_SCROLL_MAX_STEP);
    assert_eq!(auto_scroll_step(900.0, 400.0), AUTO_SCROLL_MAX_STEP);
}

#[test]
fn short_viewports_never_auto_scroll() {
    assert_eq!(auto_scroll_step(0.0, AUTO_SCROLL_MARGIN), 0.0);
}

#[test]
fn band_starting_above_the_view_is_clipped_to_the_overlay() {
    assert_eq!(
        band_placement(10.0, -40.0, 100.0, 90.0, 400.0, 300.0),
        Some((10, 0, 100, 50))
    );
}

#[test]
fn band_extending_past_the_overlay_is_clipped_to_it() {
    assert_eq!(
        band_placement(350.0, 250.0, 200.0, 200.0, 400.0, 300.0),
        Some((350, 250, 50, 50))
    );
}

#[test]
fn band_entirely_outside_the_overlay_is_hidden() {
    assert_eq!(
        band_placement(-200.0, 10.0, 100.0, 50.0, 400.0, 300.0),
        None
    );
    assert_eq!(band_placement(10.0, 400.0, 100.0, 50.0, 400.0, 300.0), None);
}

#[test]
fn items_touching_the_band_are_captured() {
    let bounds = graphene::Rect::new(0.0, 100.0, 300.0, 24.0);
    assert!(intersects(&bounds, 10.0, 90.0, 40.0, 110.0));
    assert!(!intersects(&bounds, 10.0, 0.0, 40.0, 90.0));
}

#[test]
fn an_unmoved_band_still_captures_the_item_under_it() {
    let bounds = graphene::Rect::new(0.0, 100.0, 300.0, 24.0);
    assert!(intersects(&bounds, 20.0, 110.0, 20.0, 110.0));
}
