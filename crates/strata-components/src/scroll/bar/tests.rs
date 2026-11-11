//! The scroll bar's thumb mapping and show state, with no Rune runtime. What the bar looks like
//! (its widths, gaps and least thumb length) is the gallery's PNG goldens'; how it fades and
//! drags is `strata-gallery/tests/c24.rs`'s.

use strata_kit::look::Look;

use super::{Bar, BarLook, BarMode, offset_at, thumb};

/// A bar built once, as a view builds it.
fn built() -> Bar {
    let bar = Bar::new();
    let _ = bar.build::<()>("bar", "viewport", BarLook::list(&Look::fallback(false)));
    bar
}
use rune::ScrollPos;

fn pos(offset: f32, max: f32, h: f32) -> ScrollPos {
    let mut p = ScrollPos::default();
    p.offset = [0.0, offset];
    p.max = [0.0, max];
    p.viewport = [800.0, h];
    p.drawn = p.viewport;
    p
}

#[test]
fn a_drag_reaches_both_ends_of_the_content() {
    let (top, len) = thumb(0.0, 1000.0, 500.0, 500.0, 32.0);
    assert_eq!(top, 0.0);
    let (bottom, _) = thumb(1000.0, 1000.0, 500.0, 500.0, 32.0);
    assert_eq!(bottom + len, 500.0);
}

#[test]
fn a_thumb_position_maps_back_to_its_offset() {
    for offset in [0.0, 250.0, 9_999.0, 50_000.0] {
        let (top, _) = thumb(offset, 50_000.0, 600.0, 600.0, 32.0);
        let back = offset_at(top, 50_000.0, 600.0, 600.0, 32.0);
        assert!((back - offset).abs() < 0.5, "{offset} -> {top} -> {back}");
    }
    // Past the ends it clamps.
    assert_eq!(offset_at(-50.0, 1000.0, 600.0, 600.0, 32.0), 0.0);
    assert_eq!(offset_at(5_000.0, 1000.0, 600.0, 600.0, 32.0), 1000.0);
}

#[test]
fn content_that_fits_shows_no_bar() {
    // browser/scrolling: Scrollbars fade in on hover or scrolling and stay hidden at rest; the trough strengthens on hover and while dragging.
    let bar = built();
    bar.set_mode(BarMode::Always);
    bar.follow(pos(0.0, 0.0, 600.0));
    assert!(!bar.overflows());
    assert_eq!(bar.0.opacity.get(), 0.0);
}

#[test]
fn an_always_bar_shows_while_the_content_overflows() {
    let bar = built();
    bar.set_mode(BarMode::Always);
    bar.follow(pos(0.0, 5000.0, 600.0));
    assert!(bar.overflows());
    assert_eq!(bar.0.opacity.get(), 1.0);
}

#[test]
fn a_bar_never_built_shows_nothing() {
    // Review round 1: no default look reaches the screen before the view builds the bar.
    let bar = Bar::new();
    bar.set_mode(BarMode::Always);
    bar.follow(pos(0.0, 5000.0, 600.0));
    assert!(!bar.overflows());
    assert_eq!(bar.0.opacity.get(), 0.0);
    assert_eq!(bar.0.thumb.get(), [0.0; 4]);
}

#[test]
fn an_overlay_bar_is_hidden_at_rest() {
    // browser/scrolling: Scrollbars fade in on hover or scrolling and stay hidden at rest; the trough strengthens on hover and while dragging.
    let bar = built();
    bar.follow(pos(0.0, 5000.0, 600.0));
    bar.follow(pos(0.0, 5000.0, 601.0));
    assert_eq!(bar.0.opacity.get(), 0.0);
}
