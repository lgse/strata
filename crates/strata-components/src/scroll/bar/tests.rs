// SPDX-License-Identifier: MIT

use strata_kit::look::Look;

use super::{Bar, BarLook, BarMode, offset_at, thumb};

/// A bar built once, as a view builds it.
fn built() -> Bar {
    let bar = Bar::new();
    let _ = bar.build::<()>("bar", "viewport", BarLook::list(&Look::fallback(false)));
    bar
}
use rune::ScrollPos;

fn pos_in(w: f32, offset: f32, max: f32, h: f32) -> ScrollPos {
    let mut p = ScrollPos::default();
    p.offset = [0.0, offset];
    p.max = [0.0, max];
    p.viewport = [w, h];
    p.drawn = p.viewport;
    p
}

fn pos(offset: f32, max: f32, h: f32) -> ScrollPos {
    pos_in(800.0, offset, max, h)
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

#[test]
fn all_bar_looks_share_the_same_default_resting_width() {
    let look = Look::fallback(false);
    let expected = strata_kit::slots::scroll::bar_width(&look);
    for bar_look in [
        BarLook::app(&look),
        BarLook::listing(&look),
        BarLook::list(&look),
        BarLook::column(&look),
        BarLook::strip(&look),
        BarLook::app(&look).horizontal(),
    ] {
        assert_eq!(bar_look.thickness, expected);
        assert_eq!(bar_look.thumb_width, expected);
    }
}

fn assert_vertical_spans(bar: &Bar, band: (f32, f32), trough: (f32, f32), thumb_w: f32) {
    let b = bar.0.band.get();
    let t = bar.0.trough.get();
    assert_eq!((b[0], b[2]), band);
    assert_eq!(
        (b[0] + t[0], t[2], bar.0.thumb.get()[2]),
        (trough.0, trough.1, thumb_w),
    );
}

fn assert_strip_spans(bar: &Bar, band: (f32, f32), backdrop: (f32, f32), trough: (f32, f32)) {
    let b = bar.0.band.get();
    let bg = bar.0.backdrop.get();
    let t = bar.0.trough.get();
    assert_eq!((b[1], b[3]), band);
    assert_eq!((b[1] + bg[1], bg[3]), backdrop);
    assert_eq!(
        (b[1] + t[1], t[3], bar.0.thumb.get()[3]),
        (trough.0, trough.1, trough.1),
    );
}

fn assert_zero_length(bar: &Bar, len_idx: usize) {
    assert!(!bar.overflows());
    assert_eq!(
        (
            bar.0.band.get()[len_idx],
            bar.0.backdrop.get()[len_idx],
            bar.0.trough.get()[len_idx],
        ),
        (0.0, 0.0, 0.0),
    );
}

struct RuntimeHost {
    bar: Bar,
    look: Look,
}

fn hover_runtime(hover_width: Option<f32>) -> rune::Runtime<RuntimeHost> {
    let mut look = Look::fallback(false);
    look.animations = false;
    look.slots.scroll.hover_width = hover_width;
    let bar = Bar::new();
    bar.set_mode(BarMode::Always);
    let mut rt = rune::Runtime::new(
        RuntimeHost { bar, look },
        |h: &RuntimeHost, _: &mut rune::Cx<RuntimeHost>| {
            let content = rune::col().w_full().h(2000.0);
            let viewport = rune::col()
                .key("vp")
                .w_full()
                .h_full()
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .on_scroll_offset(h.bar.watch())
                .child(content);
            let bar_el = h
                .bar
                .build::<RuntimeHost>("bar", "vp", BarLook::app(&h.look));
            rune::col()
                .w_full()
                .h_full()
                .hit()
                .cursor(rune::Cursor::Grab)
                .child(super::overlay(viewport, [bar_el]))
        },
        (300.0, 200.0),
        rune::Env::default(),
    );
    for _ in 0..4 {
        rt.frame(0.016);
    }
    rt
}

#[test]
fn hover_expands_bar_inward_tracks_scroll_and_contracts_on_leave() {
    let mut look = Look::fallback(false);
    look.slots.scroll.hover_width = Some(16.0);
    let bar = Bar::new();
    let _ = bar.build::<()>("bar", "viewport", BarLook::listing(&look));
    bar.follow(pos_in(300.0, 0.0, 5000.0, 600.0));
    assert_vertical_spans(&bar, (284.0, 16.0), (296.0, 4.0), 4.0);

    bar.0.hover(true);
    assert_vertical_spans(&bar, (284.0, 16.0), (284.0, 16.0), 16.0);
    let hovered_thumb_y = bar.0.thumb.get()[1];

    // Scroll tracking updates thumb position immediately while staying expanded.
    bar.follow(pos_in(300.0, 2500.0, 5000.0, 600.0));
    let scrolled_thumb = bar.0.thumb.get();
    assert!(scrolled_thumb[1] > hovered_thumb_y);
    assert_eq!(scrolled_thumb[2], 16.0);

    bar.0.hover(false);
    assert_vertical_spans(&bar, (284.0, 16.0), (296.0, 4.0), 4.0);
}

#[test]
fn runtime_drag_keeps_expanded_width_until_release_and_overrides_grab_cursor() {
    let mut rt = hover_runtime(Some(16.0));
    assert_vertical_spans(&rt.state.bar, (284.0, 16.0), (296.0, 4.0), 4.0);

    // Over the viewport itself, the parent's Grab cursor applies.
    rt.pointer_move(100.0, 100.0);
    rt.frame(0.016);
    assert_eq!(rt.cursor(), rune::Cursor::Grab);

    // Entering the expanded hit zone (284..300), outside the 4 px collapsed bar (296..300),
    // immediately triggers hover expansion and switches the cursor to Default.
    rt.pointer_move(286.0, 20.0);
    rt.frame(0.016);
    assert_vertical_spans(&rt.state.bar, (284.0, 16.0), (284.0, 16.0), 16.0);
    assert_eq!(rt.cursor(), rune::Cursor::Default);

    // Starting a drag inside the expanded hit zone and dragging away keeps the bar expanded until release.
    rt.pointer_down(286.0, 20.0);
    rt.frame(0.016);
    rt.pointer_move(100.0, 100.0);
    rt.frame(0.016);
    assert!(rt.state.bar.is_dragging());
    assert_vertical_spans(&rt.state.bar, (284.0, 16.0), (284.0, 16.0), 16.0);

    rt.pointer_up(100.0, 100.0);
    rt.frame(0.016);
    assert!(!rt.state.bar.is_dragging());
    assert_vertical_spans(&rt.state.bar, (284.0, 16.0), (296.0, 4.0), 4.0);
}

#[test]
fn hover_expansion_preserves_custom_thumb_inset() {
    let mut look = Look::fallback(false);
    look.slots.scroll.listing_width = Some(9.0);
    look.slots.scroll.listing_thumb_width = Some(7.0);
    look.slots.scroll.hover_width = Some(16.0);
    let bar = Bar::new();
    let _ = bar.build::<()>("bar", "viewport", BarLook::listing(&look));
    bar.follow(pos(0.0, 5000.0, 600.0));

    bar.0.hover(true);
    assert_eq!(bar.0.trough.get()[2], 16.0);
    assert_eq!(bar.0.thumb.get()[2], 14.0);
    assert_eq!(bar.0.thumb.get()[0], 1.0);
}

#[test]
fn horizontal_bar_expands_upward_from_bottom_edge() {
    let mut look = Look::fallback(false);
    look.slots.scroll.hover_width = Some(14.0);
    let bar = Bar::new();
    let _ = bar.build::<()>("bar", "viewport", BarLook::strip(&look));
    let mut p = ScrollPos::default();
    p.offset = [0.0, 0.0];
    p.max = [2000.0, 0.0];
    p.viewport = [800.0, 400.0];
    p.drawn = p.viewport;
    bar.follow(p);
    assert_strip_spans(&bar, (380.0, 20.0), (390.0, 10.0), (393.0, 4.0));

    bar.0.hover(true);
    assert_strip_spans(&bar, (380.0, 20.0), (380.0, 20.0), (383.0, 14.0));

    bar.release();
    assert_strip_spans(&bar, (380.0, 20.0), (390.0, 10.0), (393.0, 4.0));
}

#[test]
fn non_overflowing_expandable_bar_has_zero_hit_length() {
    let mut look = Look::fallback(false);
    look.slots.scroll.hover_width = Some(16.0);

    let vertical = Bar::new();
    let _ = vertical.build::<()>("vbar", "viewport", BarLook::app(&look));
    vertical.follow(pos_in(300.0, 0.0, 0.0, 200.0));
    assert_zero_length(&vertical, 3);

    let horizontal = Bar::new();
    let _ = horizontal.build::<()>("hbar", "viewport", BarLook::strip(&look));
    let mut p = ScrollPos::default();
    p.viewport = [800.0, 400.0];
    p.drawn = p.viewport;
    horizontal.follow(p);
    assert_zero_length(&horizontal, 2);
}

#[test]
fn unset_or_narrower_hover_thickness_keeps_rest_width() {
    // Default (None): hovering leaves width unchanged and band matches rest width.
    let bar = built();
    bar.follow(pos(0.0, 5000.0, 600.0));
    let rest_band = bar.0.band.get();
    assert_eq!(rest_band[2], 4.0);
    assert_eq!(bar.0.trough.get()[0], 0.0);
    bar.0.hover(true);
    assert_eq!(bar.0.band.get(), rest_band);

    // Narrower or non-finite hover_thickness never shrinks or corrupts a bar.
    for hover_width in [Some(6.0), Some(f32::NAN), Some(f32::INFINITY)] {
        let mut look = Look::fallback(false);
        look.slots.scroll.listing_width = Some(9.0);
        look.slots.scroll.listing_thumb_width = Some(7.0);
        look.slots.scroll.hover_width = hover_width;
        let listing_bar = Bar::new();
        let _ = listing_bar.build::<()>("bar", "viewport", BarLook::listing(&look));
        listing_bar.follow(pos(0.0, 5000.0, 600.0));
        assert_eq!(listing_bar.0.band.get()[2], 9.0);
        listing_bar.0.hover(true);
        assert_eq!(listing_bar.0.band.get()[2], 9.0);
        assert_eq!(listing_bar.0.trough.get()[2], 9.0);
        assert_eq!(listing_bar.0.thumb.get()[2], 7.0);
    }

    // Live restyle while hovered updates both hit band and visual width immediately.
    let mut look = Look::fallback(false);
    look.slots.scroll.hover_width = Some(12.0);
    let live_bar = Bar::new();
    let _ = live_bar.build::<()>("bar", "viewport", BarLook::app(&look));
    live_bar.follow(pos(0.0, 5000.0, 600.0));
    live_bar.0.hover(true);
    assert_eq!(live_bar.0.band.get()[2], 12.0);
    assert_eq!(live_bar.0.trough.get()[2], 12.0);

    look.slots.scroll.hover_width = Some(20.0);
    let _ = live_bar.build::<()>("bar", "viewport", BarLook::app(&look));
    assert_eq!(live_bar.0.band.get()[2], 20.0);
    assert_eq!(live_bar.0.trough.get()[2], 20.0);

    look.slots.scroll.hover_width = None;
    let _ = live_bar.build::<()>("bar", "viewport", BarLook::app(&look));
    assert_eq!(live_bar.0.band.get()[2], 4.0);
    assert_eq!(live_bar.0.trough.get()[2], 4.0);
}
