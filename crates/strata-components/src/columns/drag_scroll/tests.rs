//! The speed curve and the machine, against Strata's `drag_scroll.rs` and its tests.

use std::time::Duration;

use super::speed::{
    EDGE_MARGIN, MAX_HORIZONTAL_SPEED, MAX_VERTICAL_SPEED, RAMP, band_direction, edge_direction,
    scroll_speed,
};
use super::{ColumnGeometry, DragScroll, Scroll, StripGeometry};
use crate::contract::{Point, Rect};

/// A strip 900 wide at the window's left, scrolled `offset` of `max`, of three 300-wide
/// columns whose listings are 600 tall, each scrolled `listing` of 2,000.
fn strip(offset: f32, max: f32, listing: f32) -> StripGeometry {
    StripGeometry {
        strip: Rect::new(0.0, 40.0, 900.0, 600.0),
        offset,
        max,
        columns: (0..3)
            .map(|i| {
                let left = i as f32 * 300.0;
                ColumnGeometry {
                    left,
                    right: left + 300.0,
                    listing: Rect::new(left, 80.0, 300.0, 560.0),
                    offset: listing,
                    max: 2_000.0,
                }
            })
            .collect(),
    }
}

#[test]
fn the_speed_peaks_within_16_points_of_the_edge_at_720_and_520() {
    // operations/drag-and-drop/column-autoscroll: Scroll speed peaks within 16 px of the edge,
    // at 720 px/s horizontally and 520 px/s vertically.
    let full = RAMP * 2;
    for at in [0.0, 8.0, 16.0] {
        assert_eq!(edge_direction(at, 900.0), -1.0, "{at}");
        assert_eq!(edge_direction(900.0 - at, 900.0), 1.0, "{at}");
    }
    assert!(edge_direction(30.0, 900.0).abs() < 1.0);
    assert_eq!(
        edge_direction(EDGE_MARGIN, 900.0),
        0.0,
        "44 from the edge: nothing"
    );
    assert_eq!(edge_direction(450.0, 900.0), 0.0);
    assert_eq!(scroll_speed(1.0, full, MAX_HORIZONTAL_SPEED), 720.0);
    assert_eq!(scroll_speed(-1.0, full, MAX_VERTICAL_SPEED), -520.0);
    // A span too short for two margins never scrolls.
    assert_eq!(edge_direction(5.0, 80.0), 0.0);
    assert_eq!(band_direction(10.0, 100.0, 500.0), 0.0, "above the listing");
}

#[test]
fn the_speed_starts_at_35_percent_and_is_full_after_350_ms() {
    // operations/drag-and-drop/column-autoscroll: Scrolling starts at 35% of that speed and
    // reaches full speed after 350 ms in the same direction.
    assert_eq!(scroll_speed(1.0, Duration::ZERO, 1000.0), 350.0);
    let half = scroll_speed(1.0, RAMP / 2, 1000.0);
    assert!(half > 350.0 && half < 1000.0, "{half}");
    assert_eq!(scroll_speed(1.0, RAMP, 1000.0), 1000.0);
    assert_eq!(scroll_speed(1.0, RAMP * 3, 1000.0), 1000.0);
}

#[test]
fn a_held_drag_glides_toward_the_edge_and_its_dwell_restarts_with_a_new_direction() {
    let mut m = DragScroll::new();
    let geo = strip(300.0, 600.0, 0.0);
    assert!(m.over(Some(Point::new(5.0, 300.0))));
    let step = m.step(0.0, &geo, true);
    assert!(
        matches!(step.strip, Some(Scroll::Glide(x)) if x < 300.0),
        "{step:?}"
    );
    assert!(step.wake);
    // Animations off: one frame's travel, at the ramp's floor on the first frame (16 ms).
    let mut m = DragScroll::new();
    m.over(Some(Point::new(5.0, 300.0)));
    let step = m.step(0.0, &geo, false);
    let Some(Scroll::Instant(x)) = step.strip else {
        panic!("{step:?}");
    };
    assert!((300.0 - x - 720.0 * 0.35 * 0.016).abs() < 0.01, "{x}");
    // The other edge: the dwell starts again.
    m.over(Some(Point::new(895.0, 300.0)));
    let step = m.step(1.0, &geo, false);
    let Some(Scroll::Instant(x)) = step.strip else {
        panic!("{step:?}");
    };
    assert!(x > 300.0 && x - 300.0 < 720.0 * 0.36 * 0.05, "{x}");
}

#[test]
fn autoscroll_stops_on_leaving_the_edge_the_strip_or_the_end_and_when_the_drag_goes() {
    // operations/drag-and-drop/column-autoscroll: Autoscroll stops when the drag leaves the
    // strip, drops, or ends, or the content cannot scroll further.
    let geo = strip(300.0, 600.0, 0.0);
    let start = |m: &mut DragScroll| {
        m.over(Some(Point::new(5.0, 300.0)));
        m.step(0.0, &geo, true);
        assert!(m.scrolling());
    };
    // Leaves the edge: a dead stop where the strip is, then nothing.
    let mut m = DragScroll::new();
    start(&mut m);
    m.over(Some(Point::new(450.0, 300.0)));
    let step = m.step(0.016, &geo, true);
    assert_eq!(step.strip, Some(Scroll::Instant(300.0)));
    assert!(!step.wake);
    // Leaves the strip (below it).
    let mut m = DragScroll::new();
    start(&mut m);
    m.over(Some(Point::new(5.0, 700.0)));
    assert_eq!(
        m.step(0.016, &geo, true).strip,
        Some(Scroll::Instant(300.0))
    );
    // Drops or ends.
    let mut m = DragScroll::new();
    start(&mut m);
    assert!(m.over(None), "a frame to stop in");
    let step = m.step(0.016, &geo, true);
    assert_eq!(step.strip, Some(Scroll::Instant(300.0)));
    assert!(!m.scrolling());
    // At the end: nothing to do.
    let mut m = DragScroll::new();
    m.over(Some(Point::new(5.0, 300.0)));
    let step = m.step(0.0, &strip(0.0, 600.0, 0.0), true);
    assert_eq!(step.strip, None);
    assert!(!step.wake, "no frames while it can't move");
}

#[test]
fn a_listing_scrolls_near_its_top_or_bottom_and_only_the_one_under_the_drag() {
    let mut m = DragScroll::new();
    m.over(Some(Point::new(450.0, 635.0)));
    let step = m.step(0.0, &strip(0.0, 0.0, 100.0), true);
    assert!(
        matches!(step.listing, Some((1, Scroll::Glide(y))) if y > 100.0),
        "{step:?}"
    );
    assert_eq!(step.strip, None);
    // Its top, at its start: nothing.
    let mut m = DragScroll::new();
    m.over(Some(Point::new(450.0, 85.0)));
    assert_eq!(m.step(0.0, &strip(0.0, 0.0, 0.0), true).listing, None);
}

#[test]
fn the_column_under_the_drag_follows_the_strip_under_a_still_pointer() {
    // operations/drag-and-drop/column-autoscroll: The column under a held file drag shows an
    // accent wash and ring, including a column just revealed by autoscroll.
    let mut m = DragScroll::new();
    m.over(Some(Point::new(250.0, 300.0)));
    assert!(m.hover(&strip(0.0, 600.0, 0.0)));
    assert_eq!(m.hovered(), Some(0));
    // The strip moved 100 under the still pointer: the columns' spans moved with it.
    let mut moved = strip(100.0, 600.0, 0.0);
    for c in &mut moved.columns {
        c.left -= 100.0;
        c.right -= 100.0;
    }
    assert!(m.hover(&moved));
    assert_eq!(m.hovered(), Some(1));
    m.over(None);
    assert!(m.hover(&moved));
    assert_eq!(m.hovered(), None, "it clears when the drag goes");
}
