use std::cell::Cell;

use rune::{
    Cx, DragItem, El, Env, Key, KeyEvent, MagnifyEvent, Mods, Pixels, Runtime, WheelEvent, el,
};
use strata_kit::look::Look;

use super::zoom::{
    Mode, ZoomKey, ZoomLimits, ZoomReport, ctrl_zero_key, magnify_factor, mode_level, wheel_factor,
    zoom_about_scroll, zoom_key,
};
use super::{Interaction, PanDrag, Viewport, ViewportState};
use crate::contract::{Dnd, DragSource};

const DT: f64 = 1.0 / 60.0;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3 * a.abs().max(b.abs()).max(1.0)
}

fn near2(a: [f32; 2], b: [f32; 2]) -> bool {
    near(a[0], b[0]) && near(a[1], b[1])
}

/// A 1600 × 1000 content in a 400 × 300 frame at scale 1: fit is 0.25.
fn laid_out(mode: Mode) -> ViewportState {
    let mut st = ViewportState::new(mode);
    st.set_content(Some([1600.0, 1000.0]));
    st.set_frame([400.0, 300.0], 1.0);
    st
}

// The zoom model.

#[test]
fn fit_modes_follow_the_frame_and_one_to_one_follows_device_pixels() {
    let (frame, content) = ([400.0, 300.0], [1600.0, 1000.0]);
    assert!(near(mode_level(Mode::Fit, frame, content, 1.0, 9.0), 0.25));
    assert!(near(mode_level(Mode::Fill, frame, content, 1.0, 9.0), 0.3));
    // At 2× a point is two device pixels, so the same frame fits at twice the level.
    assert!(near(mode_level(Mode::Fit, frame, content, 2.0, 9.0), 0.5));
    assert!(near(
        mode_level(Mode::Actual, frame, content, 2.0, 9.0),
        1.0
    ));
    assert!(near(mode_level(Mode::Free, frame, content, 2.0, 9.0), 9.0));
}

#[test]
fn scale_down_never_enlarges() {
    let small = [100.0, 50.0];
    assert!(near(
        mode_level(Mode::ScaleDown, [400.0, 300.0], small, 1.0, 1.0),
        1.0
    ));
    assert!(near(
        mode_level(Mode::Fit, [400.0, 300.0], small, 1.0, 1.0),
        4.0
    ));
    assert!(near(
        mode_level(Mode::ScaleDown, [50.0, 300.0], small, 1.0, 1.0),
        0.5
    ));
}

#[test]
fn a_wheel_turned_away_zooms_in_and_a_notch_back_returns() {
    // Rune's `WheelEvent::dy` is positive when content moves down: a wheel turned away from the
    // user, which zooms in, as in browsers and Strata's PDF preview.
    assert!(
        wheel_factor(40.0) > 1.0,
        "a wheel turned away (dy > 0) zooms in"
    );
    assert!(near(wheel_factor(40.0) * wheel_factor(-40.0), 1.0));
    assert!(near(magnify_factor(0.2) * magnify_factor(-0.2), 1.0));
}

#[test]
fn zooming_a_scrolled_column_keeps_the_anchor_in_place() {
    // A column scrolled 100 down, the pointer 50 below its top: the content point under the
    // pointer is 150 in. Doubled, it is 300 in, so the column scrolls to 250.
    assert!(near(zoom_about_scroll(100.0, 50.0, 2.0), 250.0));
    assert!(near(zoom_about_scroll(250.0, 50.0, 0.5), 100.0));
}

#[test]
fn wanted_pixels_follow_the_drawn_size_up_to_the_content_and_the_cap() {
    let half = ZoomReport::new(0.5, Mode::Fit, [1600.0, 1000.0]);
    assert_eq!(half.wanted_px(8192), [800, 500]);
    let big = ZoomReport::new(4.0, Mode::Free, [1600.0, 1000.0]);
    assert_eq!(
        big.wanted_px(8192),
        [1600, 1000],
        "never more than the content's own pixels"
    );
    let pano = ZoomReport::new(1.0, Mode::Actual, [42374.0, 10529.0]);
    let [w, h] = pano.wanted_px(8192);
    assert_eq!(w, 8192);
    assert!(
        (h as f32 - 8192.0 * 10529.0 / 42374.0).abs() <= 1.0,
        "keeps its shape: {h}"
    );
}

// The keys.

fn ev(key: Key, mods: Mods) -> KeyEvent {
    KeyEvent { key, mods }
}

fn text(t: &str) -> Key {
    Key::Text(t.into())
}

#[test]
fn plain_plus_minus_and_zero_zoom_and_reset() {
    let none = Mods::default();
    let shift = Mods {
        shift: true,
        ..none
    };
    assert_eq!(zoom_key(&ev(text("+"), shift)), Some(ZoomKey::In));
    assert_eq!(zoom_key(&ev(text("="), none)), Some(ZoomKey::In));
    assert_eq!(zoom_key(&ev(text("-"), none)), Some(ZoomKey::Out));
    assert_eq!(zoom_key(&ev(text("0"), none)), Some(ZoomKey::Reset));
    assert_eq!(zoom_key(&ev(Key::Right, none)), Some(ZoomKey::Pan(1, 0)));
    assert_eq!(zoom_key(&ev(Key::Up, none)), Some(ZoomKey::Pan(0, -1)));
}

#[test]
fn ctrl_zero_resets_but_not_with_alt_or_super_and_ctrl_plus_is_left_to_text_size() {
    // Off macOS Ctrl is the primary modifier: Rune sends Ctrl+0 as `Command('0')` with both.
    let ctrl = Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    };
    assert_eq!(
        ctrl_zero_key(&ev(Key::Command('0'), ctrl)),
        Some(ZoomKey::Reset)
    );
    // A keypad 0 with Num Lock types "0".
    assert_eq!(ctrl_zero_key(&ev(text("0"), ctrl)), Some(ZoomKey::Reset));
    // ⌘0 on macOS.
    let cmd = Mods {
        cmd: true,
        logo: true,
        ..Mods::default()
    };
    assert_eq!(
        ctrl_zero_key(&ev(Key::Command('0'), cmd)),
        Some(ZoomKey::Reset)
    );
    let alt = Mods { alt: true, ..ctrl };
    let sup = Mods { logo: true, ..ctrl };
    assert_eq!(zoom_key(&ev(Key::Command('0'), alt)), None);
    assert_eq!(zoom_key(&ev(Key::Command('0'), sup)), None);
    assert_eq!(zoom_key(&ev(Key::Command('='), ctrl)), None);
    assert_eq!(zoom_key(&ev(Key::Command('-'), ctrl)), None);
    assert_eq!(zoom_key(&ev(Key::Right, ctrl)), None);
}

// The state.

#[test]
fn nothing_zooms_until_the_content_and_the_frame_are_known() {
    let mut st = ViewportState::default();
    assert_eq!(st.level(), None);
    assert_eq!(st.zoom_step(1), None);
    st.set_content(Some([100.0, 100.0]));
    assert_eq!(st.level(), None);
    assert!(
        st.set_frame([200.0, 200.0], 1.0).is_some(),
        "the first layout reports the level"
    );
    assert_eq!(st.level(), Some(2.0));
}

#[test]
fn zooming_keeps_the_content_point_under_the_pointer() {
    let mut st = laid_out(Mode::Fit);
    st.zoom_to(1.0, None);
    let p = [130.0, 90.0];
    let before = st.content_at(p).expect("laid out");
    let report = st.zoom_by(1.5, Some(p)).expect("the level changed");
    assert!(near(report.level, 1.5));
    assert_eq!(st.mode(), Mode::Free);
    assert!(near2(st.content_at(p).expect("laid out"), before));
}

#[test]
fn zooming_out_past_fit_stops_at_the_lower_limit_or_fit() {
    let mut st = laid_out(Mode::Fit);
    // Fit (0.25) is above the 0.1 floor: zooming out goes to 0.1, centred.
    st.zoom_by(0.01, Some([10.0, 10.0]));
    assert!(near(st.level().expect("level"), 0.1));
    assert_eq!(
        st.pan(),
        [0.0, 0.0],
        "a content smaller than the frame is centred"
    );

    let mut huge = ViewportState::default();
    huge.set_content(Some([40000.0, 10000.0]));
    huge.set_frame([400.0, 300.0], 1.0);
    let fit = huge.level().expect("level");
    assert!(fit < 0.1);
    assert_eq!(
        huge.zoom_by(0.5, None),
        None,
        "fit is the floor when it's below the limits"
    );
    assert!(near(huge.range().expect("range").min, fit));
}

#[test]
fn minus_at_a_huge_images_fit_does_nothing() {
    let mut huge = ViewportState::default();
    huge.set_content(Some([40000.0, 10000.0]));
    huge.set_frame([400.0, 300.0], 1.0);
    let fit = huge.level().expect("level");
    // Fit (0.01) is below the first step (0.1): `-` must not jump up to it.
    assert_eq!(huge.zoom_step(-1), None);
    assert!(near(huge.level().expect("level"), fit));
    assert!(huge.zoom_step(1).expect("in").level > fit);
}

#[test]
fn plus_and_ctrl_wheel_at_a_tiny_images_fit_do_nothing() {
    let mut tiny = ViewportState::default();
    tiny.set_content(Some([8.0, 8.0]));
    tiny.set_frame([400.0, 300.0], 1.0);
    let fit = tiny.level().expect("level");
    // Fit (37.5) is above the last step (32): zooming in must not drop to it.
    assert!(fit > 32.0);
    assert_eq!(tiny.zoom_step(1), None);
    assert_eq!(tiny.zoom_by(wheel_factor(40.0), None), None);
    assert!(near(tiny.level().expect("level"), fit));
    assert!(near(tiny.range().expect("range").max, fit));
    assert!(tiny.zoom_step(-1).expect("out").level < fit);
}

#[test]
fn steps_go_to_the_next_step_from_between_steps_and_stop_at_the_ends() {
    let mut st = laid_out(Mode::Fit);
    assert!(near(st.zoom_step(1).expect("in").level, 1.0 / 3.0));
    assert!(near(st.zoom_step(-1).expect("out").level, 0.25));
    st.zoom_to(32.0, None);
    assert_eq!(st.zoom_step(1), None, "already at the top");
}

#[test]
fn custom_limits_hold_the_zoom() {
    let mut st = ViewportState::default().with_limits(ZoomLimits::new(0.25, 2.0));
    st.set_content(Some([1600.0, 1000.0]));
    st.set_frame([400.0, 300.0], 1.0);
    st.zoom_by(100.0, None);
    assert!(near(st.level().expect("level"), 2.0));
}

#[test]
fn pans_stop_at_the_content_edges() {
    let mut st = laid_out(Mode::Fit);
    st.zoom_to(1.0, None);
    // 1600 × 1000 in 400 × 300: 600 and 350 of reach.
    assert!(st.pan_by([-5000.0, 5000.0]));
    assert_eq!(st.pan(), [-600.0, 350.0]);
    assert!(!st.pan_by([-10.0, 10.0]), "already at the edges");
    let right_edge = st.content_at([400.0, 0.0]).expect("laid out");
    assert!(
        near2(right_edge, [1600.0, 0.0]),
        "the content's corner is at the frame's: {right_edge:?}"
    );
}

#[test]
fn a_resize_refits_in_fit_and_keeps_the_level_in_free() {
    let mut st = laid_out(Mode::Fit);
    let report = st
        .set_frame([800.0, 600.0], 1.0)
        .expect("fit follows the frame");
    assert!(near(report.level, 0.5));
    assert_eq!(
        st.set_frame([800.0, 600.0], 1.0),
        None,
        "the same frame reports nothing"
    );

    st.zoom_to(1.0, None);
    st.pan_by([-1000.0, 0.0]);
    assert_eq!(st.pan(), [-400.0, 0.0]);
    assert_eq!(
        st.set_frame([1200.0, 600.0], 1.0),
        None,
        "Free keeps its level"
    );
    assert_eq!(
        st.pan(),
        [-200.0, 0.0],
        "and its pan moves in to the new edges"
    );
}

#[test]
fn new_content_starts_over_and_the_same_size_keeps_the_zoom() {
    let mut st = laid_out(Mode::Fit);
    st.zoom_to(2.0, None);
    st.pan_by([-100.0, 0.0]);
    assert_eq!(st.set_content(Some([1600.0, 1000.0])), None);
    assert_eq!((st.mode(), st.pan()), (Mode::Free, [-100.0, 0.0]));
    // A new size refits, and reports it for the host to forward: no layout will.
    let report = st.set_content(Some([800.0, 800.0])).expect("a new level");
    assert_eq!((st.mode(), st.pan()), (Mode::Fit, [0.0, 0.0]));
    assert!(near(report.level, 300.0 / 800.0));
    assert_eq!(report.content, [800.0, 800.0]);
}

#[test]
fn new_content_before_the_first_layout_leaves_the_report_to_it() {
    let mut st = ViewportState::default();
    assert_eq!(st.set_content(Some([800.0, 800.0])), None);
    assert!(st.set_frame([400.0, 300.0], 1.0).is_some());
}

#[test]
fn reset_returns_to_the_default_mode_centred() {
    let mut st = laid_out(Mode::ScaleDown);
    st.zoom_to(3.0, None);
    st.pan_by([50.0, 50.0]);
    st.reset();
    assert_eq!((st.mode(), st.pan()), (Mode::ScaleDown, [0.0, 0.0]));
    assert_eq!(ViewportState::new(Mode::Free).default_mode(), Mode::Fit);
}

// The element, in a runtime.

thread_local! {
    static VIEWS: Cell<u32> = const { Cell::new(0) };
    static DRAG_OUTS: Cell<u32> = const { Cell::new(0) };
}

struct App {
    view: ViewportState,
    pixels: Pixels,
    look: Look,
    interaction: Interaction,
    /// A file drag source on the viewport, as Strata's preview picture has.
    drag_source: bool,
    /// Presses heard around the viewport (C13's click to play).
    outer_presses: u32,
    reports: Vec<ZoomReport>,
    outer_wheels: u32,
    outer_pinches: u32,
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    VIEWS.with(|v| v.set(v.get() + 1));
    let mut viewport = Viewport::new(&s.view, |s: &mut App| &mut s.view)
        .key("vp")
        .pixels(&s.pixels)
        .label("photo.png")
        .transparent(true)
        .interaction(s.interaction)
        .pan_drag(PanDrag::WhenOverflowing)
        .on_zoom(|s: &mut App, r, _tx| s.reports.push(r));
    if s.drag_source {
        // A binder that counts the drags it would start.
        let dnd = Dnd::custom(
            |el, _| el,
            |el: El, _source| {
                el.drag_out(|_: &App| {
                    DRAG_OUTS.with(|d| d.set(d.get() + 1));
                    Some(DragItem::new().text("photo.png"))
                })
            },
        );
        viewport = viewport.drag_source(dnd, DragSource::new(|_: &App| None));
    }
    let around = el()
        .key("around")
        .size(400.0, 300.0)
        .on_press(|s: &mut App| s.outer_presses += 1)
        .child(viewport.build(&s.look, cx));
    // In a window off macOS, this `on_magnify` around a Full viewport would take its Ctrl+wheel
    // (module docs); headless, Ctrl+wheel always goes to `on_wheel`.
    el().key("root")
        .size(400.0, 300.0)
        .on_wheel(|s: &mut App, _e, _tx| {
            s.outer_wheels += 1;
            true
        })
        .on_magnify(|s: &mut App, _e, _tx| {
            s.outer_pinches += 1;
            true
        })
        .child(around)
}

fn runtime(interaction: Interaction, drag_source: bool) -> Runtime<App> {
    let pixels = Pixels::new();
    pixels.set(16, 10, vec![200; 16 * 10 * 4]);
    let mut view_state = ViewportState::default();
    view_state.set_content(Some([1600.0, 1000.0]));
    let app = App {
        view: view_state,
        pixels,
        look: Look::fallback(false),
        interaction,
        drag_source,
        outer_presses: 0,
        reports: Vec::new(),
        outer_wheels: 0,
        outer_pinches: 0,
    };
    let mut rt = Runtime::new(app, view, (400.0, 300.0), Env::new(false, 1.0));
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<App>) {
    for _ in 0..30 {
        rt.frame(DT);
    }
}

fn views() -> u32 {
    VIEWS.with(Cell::get)
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}

fn drag(rt: &mut Runtime<App>, from: (f32, f32), by: (f32, f32)) {
    rt.pointer_move(from.0, from.1);
    rt.pointer_down(from.0, from.1);
    for i in 1..=5 {
        let t = i as f32 / 5.0;
        rt.pointer_move(from.0 + by.0 * t, from.1 + by.1 * t);
        rt.frame(DT);
    }
    rt.pointer_up(from.0 + by.0, from.1 + by.1);
    settle(rt);
}

#[test]
fn the_layout_settles_and_reports_the_fit_once() {
    let mut rt = runtime(Interaction::Full, false);
    assert_eq!(rt.state.view.frame(), Some([400.0, 300.0]));
    assert_eq!(
        rt.state.reports.len(),
        1,
        "the first layout reports the fit"
    );
    assert!(near(rt.state.reports[0].level, 0.25));
    let before = views();
    settle(&mut rt);
    assert_eq!(views(), before, "nothing re-runs the view once laid out");
}

#[test]
fn pixels_update_in_place_without_running_the_view() {
    let mut rt = runtime(Interaction::Fixed, false);
    let before = views();
    rt.state.pixels.set(16, 10, vec![10; 16 * 10 * 4]);
    settle(&mut rt);
    // A frame of another shape too: the element's size comes from the content's, not the image's.
    rt.state.pixels.set(32, 10, vec![10; 32 * 10 * 4]);
    settle(&mut rt);
    assert_eq!(views(), before);
}

#[test]
fn panning_by_drag_and_wheel_skips_the_view() {
    let mut rt = runtime(Interaction::Full, false);
    // Debug builds re-run the view after a late live-only event to check it; this test counts
    // the runs, as Rune's README says to.
    rt.check_live_only(false);
    rt.update(|s, _| {
        s.view.zoom_to(1.0, None);
    });
    settle(&mut rt);
    // A press focuses the viewport, and Rune commits for that: count from after the press.
    rt.pointer_move(200.0, 150.0);
    rt.pointer_down(200.0, 150.0);
    settle(&mut rt);
    let (before, live) = (views(), rt.live_only_events());
    for i in 1..=5 {
        rt.pointer_move(200.0 - 12.0 * i as f32, 150.0 - 6.0 * i as f32);
        rt.frame(DT);
    }
    rt.pointer_up(140.0, 120.0);
    settle(&mut rt);
    assert_eq!(rt.state.view.pan(), [-60.0, -30.0]);
    rt.wheel_event(WheelEvent::new(-20.0, 0.0).at(200.0, 150.0));
    settle(&mut rt);
    assert_eq!(rt.state.view.pan(), [-80.0, -30.0]);
    assert_eq!(views(), before, "pans only move the live offset");
    assert!(rt.live_only_events() > live);
    assert_eq!(rt.state.outer_wheels, 0);
}

#[test]
fn a_drag_at_fit_drags_the_file_out_and_a_drag_above_fit_pans() {
    let mut rt = runtime(Interaction::Full, true);
    DRAG_OUTS.with(|d| d.set(0));
    drag(&mut rt, (200.0, 150.0), (60.0, 0.0));
    assert_eq!(
        DRAG_OUTS.with(Cell::get),
        1,
        "at fit the host's drag source drags"
    );
    assert_eq!(rt.state.view.pan(), [0.0, 0.0]);

    rt.update(|s, _| {
        s.view.zoom_to(1.0, None);
    });
    settle(&mut rt);
    drag(&mut rt, (200.0, 150.0), (60.0, 0.0));
    assert_eq!(DRAG_OUTS.with(Cell::get), 1, "above fit nothing drags out");
    assert_eq!(rt.state.view.pan(), [60.0, 0.0]);
}

#[test]
fn a_plain_wheel_with_nothing_to_pan_scrolls_on() {
    let mut rt = runtime(Interaction::Full, false);
    rt.wheel_event(WheelEvent::new(0.0, -40.0).at(200.0, 150.0));
    settle(&mut rt);
    assert_eq!(rt.state.outer_wheels, 1);
}

#[test]
fn a_fixed_viewport_leaves_ctrl_wheel_and_pinches_to_the_app() {
    let mut rt = runtime(Interaction::Fixed, false);
    rt.wheel_event(WheelEvent::new(0.0, 40.0).at(200.0, 150.0).mods(ctrl()));
    rt.magnify_event(MagnifyEvent::new(0.2).at(200.0, 150.0));
    settle(&mut rt);
    assert_eq!((rt.state.outer_wheels, rt.state.outer_pinches), (1, 1));
    assert!(
        near(rt.state.view.level().expect("level"), 0.25),
        "the viewport didn't zoom"
    );
}

#[test]
fn ctrl_wheel_turned_away_zooms_in() {
    let mut rt = runtime(Interaction::Full, false);
    rt.wheel_event(WheelEvent::new(0.0, 40.0).at(200.0, 150.0).mods(ctrl()));
    settle(&mut rt);
    assert_eq!(rt.state.outer_wheels, 0);
    let level = rt.state.view.level().expect("level");
    assert!(level > 0.25, "zoomed in: {level}");
    assert!(near(level, 0.25 * wheel_factor(40.0)), "{level}");
    assert_eq!(rt.state.reports.len(), 2, "the fit, then the zoom");
}

#[test]
fn a_pinch_zooms_on_macos_and_reaches_the_app_elsewhere() {
    let mut rt = runtime(Interaction::Full, false);
    rt.magnify_event(MagnifyEvent::new(0.2).at(200.0, 150.0));
    settle(&mut rt);
    let level = rt.state.view.level().expect("level");
    if cfg!(target_os = "macos") {
        assert!(near(level, 0.25 * magnify_factor(0.2)), "{level}");
        assert_eq!(rt.state.outer_pinches, 0);
    } else {
        // Off macOS a pinch arrives as Ctrl+wheel, so the viewport has no `on_magnify`.
        assert!(near(level, 0.25));
        assert_eq!(rt.state.outer_pinches, 1);
    }
}

/// Holds off macOS in a window too: the viewport attaches no `on_magnify` there, so Rune
/// doesn't turn the Ctrl+wheel into a pinch, and `on_wheel` sees Shift.
#[test]
fn ctrl_shift_wheel_is_left_to_the_app() {
    let mut rt = runtime(Interaction::Full, false);
    let mods = Mods {
        shift: true,
        ..ctrl()
    };
    rt.wheel_event(WheelEvent::new(0.0, 40.0).at(200.0, 150.0).mods(mods));
    settle(&mut rt);
    assert_eq!(rt.state.outer_wheels, 1);
}

#[test]
fn a_new_display_scale_rescales_before_a_zoom() {
    let mut rt = runtime(Interaction::Full, false);
    // The window moves to a 2× display: the same frame now fits at twice the level.
    rt.set_viewport((400.0, 300.0), 2.0);
    settle(&mut rt);
    rt.wheel_event(WheelEvent::new(0.0, 40.0).at(200.0, 150.0).mods(ctrl()));
    settle(&mut rt);
    let levels: Vec<f32> = rt.state.reports.iter().map(|r| r.level).collect();
    let last = *levels.last().expect("reports");
    assert!(near(last, 0.5 * wheel_factor(40.0)), "{levels:?}");
    assert!(
        levels.iter().any(|&l| near(l, 0.5)),
        "the new fit is reported: {levels:?}"
    );
}

/// A window moved to a 2× display: no input, no layout change in points.
fn rescale_with_no_input(interaction: Interaction) {
    // Fit: 0.25 at 1×, so 0.5 at 2×, reported.
    let mut rt = runtime(interaction, false);
    let reports = rt.state.reports.len();
    rt.set_viewport((400.0, 300.0), 2.0);
    settle(&mut rt);
    assert_eq!(rt.state.view.scale(), 2.0, "{interaction:?}");
    assert!(
        near(rt.state.view.level().expect("level"), 0.5),
        "{interaction:?}"
    );
    assert_eq!(rt.state.reports.len(), reports + 1, "{interaction:?}");
    assert!(near(rt.state.reports[reports].level, 0.5));
    if interaction == Interaction::Full {
        let value = rt.accessibility_json().to_string();
        assert!(value.contains("\"50%\""), "the image reads 50%");
    }

    // 1:1 panned to a corner: 1600 × 1000 points at 1×, 800 × 500 at 2×, so the pan's reach
    // falls from (600, 350) to (200, 100).
    let mut rt = runtime(interaction, false);
    rt.update(|s, _| {
        s.view.set_mode(Mode::Actual);
        s.view.pan_by([-5000.0, 5000.0]);
    });
    settle(&mut rt);
    assert_eq!(rt.state.view.pan(), [-600.0, 350.0]);
    rt.set_viewport((400.0, 300.0), 2.0);
    settle(&mut rt);
    assert_eq!(rt.state.view.pan(), [-200.0, 100.0], "{interaction:?}");
    let content = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("vp-content"))
        .expect("the content")
        .rect;
    // Panned left and down to the content's top-right corner, which is the frame's: no gap.
    assert!(
        near(content[0] + content[2], 400.0) && near(content[1], 0.0),
        "{content:?}"
    );
}

#[test]
fn a_scale_change_alone_resyncs_a_full_viewport() {
    rescale_with_no_input(Interaction::Full);
}

#[test]
fn a_scale_change_alone_resyncs_a_fixed_viewport() {
    rescale_with_no_input(Interaction::Fixed);
}

/// 2× for a single frame, then back to 1×: the second change still resyncs. With one rescale
/// key for every scale, the element mounted for 2× is still in the tree when 1× asks for it, so
/// Rune keeps it, it reports no fresh layout, and the state stays at 2×.
fn rescale_round_trip(interaction: Interaction) {
    let mut rt = runtime(interaction, false);
    rt.set_viewport((400.0, 300.0), 2.0);
    rt.frame(DT);
    rt.set_viewport((400.0, 300.0), 1.0);
    settle(&mut rt);
    assert_eq!(rt.state.view.scale(), 1.0, "{interaction:?}");
    assert!(
        near(rt.state.view.level().expect("level"), 0.25),
        "{interaction:?}: {:?}",
        rt.state.view.level()
    );
    let last = rt.state.reports.last().expect("reports").level;
    assert!(near(last, 0.25), "{interaction:?}: last report {last}");
}

#[test]
fn a_scale_round_trip_in_one_frame_resyncs_a_full_viewport() {
    rescale_round_trip(Interaction::Full);
}

#[test]
fn a_scale_round_trip_in_one_frame_resyncs_a_fixed_viewport() {
    rescale_round_trip(Interaction::Fixed);
}

#[test]
fn pans_pass_rune_s_live_only_check() {
    let mut rt = runtime(Interaction::Full, false);
    rt.update(|s, _| {
        s.view.zoom_to(1.0, None);
    });
    settle(&mut rt);
    // Each step is held past the check's quarter second, so Rune re-runs the view after each
    // live-only event and compares it with what's on screen.
    rt.pointer_move(200.0, 150.0);
    rt.pointer_down(200.0, 150.0);
    settle(&mut rt);
    for i in 1..=5 {
        rt.pointer_move(200.0 - 12.0 * i as f32, 150.0);
        settle(&mut rt);
    }
    rt.pointer_up(140.0, 150.0);
    settle(&mut rt);
    rt.wheel_event(WheelEvent::new(-20.0, 0.0).at(200.0, 150.0));
    settle(&mut rt);
    rt.key(KeyEvent {
        key: Key::Left,
        mods: Mods::default(),
    });
    settle(&mut rt);
    assert_eq!(rt.state.view.pan(), [-60.0 - 20.0 + 24.0, 0.0]);
    assert_eq!(rt.live_only_warnings(), &[] as &[String]);
}

thread_local! {
    static SCROLL_VIEWS: Cell<u32> = const { Cell::new(0) };
}

/// The viewport in a scroll view, under a tall tail.
fn scroll_view(s: &App, cx: &mut Cx<App>) -> El {
    SCROLL_VIEWS.with(|v| v.set(v.get() + 1));
    let viewport = Viewport::new(&s.view, |s: &mut App| &mut s.view)
        .key("vp")
        .pixels(&s.pixels)
        .build(&s.look, cx);
    rune::col()
        .key("root")
        .size(400.0, 300.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            rune::col()
                .key("stack")
                .w_full()
                .child(el().key("box").w_full().h(300.0).child(viewport))
                .child(el().key("tail").w_full().h(900.0)),
        )
}

#[test]
fn scrolling_past_a_viewport_runs_no_view() {
    let pixels = Pixels::new();
    pixels.set(16, 10, vec![200; 16 * 10 * 4]);
    let mut view = ViewportState::default();
    view.set_content(Some([1600.0, 1000.0]));
    let app = App {
        view,
        pixels,
        look: Look::fallback(false),
        interaction: Interaction::Full,
        drag_source: false,
        outer_presses: 0,
        reports: Vec::new(),
        outer_wheels: 0,
        outer_pinches: 0,
    };
    let mut rt = Runtime::new(app, scroll_view, (400.0, 300.0), Env::new(false, 1.0));
    rt.check_live_only(false);
    settle(&mut rt);
    let before = SCROLL_VIEWS.with(Cell::get);
    // The viewport moves with the scroll, so its layout handler hears every step, but its size
    // stays: nothing the view reads changes.
    rt.pointer_move(200.0, 150.0);
    for _ in 0..10 {
        rt.wheel_event(WheelEvent::new(0.0, -20.0).at(200.0, 150.0));
        rt.frame(DT);
    }
    settle(&mut rt);
    let frame = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("vp"))
        .expect("the viewport")
        .rect;
    assert!(frame[1] < 0.0, "the viewport scrolled up: {frame:?}");
    assert_eq!(SCROLL_VIEWS.with(Cell::get), before);
}

#[test]
fn a_press_on_a_fixed_viewport_reaches_the_element_around_it() {
    let mut rt = runtime(Interaction::Fixed, false);
    rt.pointer_move(200.0, 150.0);
    rt.pointer_down(200.0, 150.0);
    rt.pointer_up(200.0, 150.0);
    settle(&mut rt);
    assert_eq!(rt.state.outer_presses, 1);
}
