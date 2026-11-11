//! The autoscroll machine, its speed curve and the collection decorator, with no Rune runtime.
//! Each test names the `browser/scrolling` bullet it checks.

use std::rc::Rc;

use rune::{PointerButton, ScrollPos, Spring};

use super::machine::{Machine, Out};
use super::{Autoscroll, AutoscrollPointer, AutoscrollSettings, Start, Target, speed};
use crate::collection::element::{
    Ctx, Effect, Flow, Hit, PointerController, PointerInput, PointerPhase, Region, Settings,
};
use crate::contract::{EntryId, EntryKind, ListEntry, ListSource, Point, Rect, UniformRows};
use crate::scroll::ScrollFeed;

const S: AutoscrollSettings = AutoscrollSettings::STRATA;

#[derive(Debug, PartialEq)]
enum Req {
    Glide(String, f32, f32),
    Jump(String, f32, f32),
    Wake,
}

#[derive(Default)]
struct Rec(Vec<Req>);

impl Out for Rec {
    fn glide(&mut self, path: &str, x: f32, y: f32, _spring: Spring) {
        self.0.push(Req::Glide(path.to_owned(), x, y));
    }
    fn jump(&mut self, path: &str, x: f32, y: f32) {
        self.0.push(Req::Jump(path.to_owned(), x, y));
    }
    fn wake(&mut self) {
        self.0.push(Req::Wake);
    }
}

fn feed(offset: [f32; 2], max: [f32; 2]) -> ScrollFeed {
    let f = ScrollFeed::new();
    let mut p = ScrollPos::default();
    p.offset = offset;
    p.max = max;
    p.viewport = [800.0, 600.0];
    p.drawn = p.viewport;
    f.set(p);
    f
}

fn start(m: &mut Machine, targets: Vec<Target>) -> bool {
    m.start(
        Start {
            owner: "list".into(),
            anchor: Point::new(400.0, 300.0),
            viewport: Rect::new(0.0, 0.0, 800.0, 600.0),
            targets,
        },
        S,
    )
}

fn running(offset_y: f32) -> (Machine, ScrollFeed) {
    let f = feed([0.0, offset_y], [0.0, 10_000.0]);
    let mut m = Machine::default();
    assert!(start(
        &mut m,
        vec![Target::vertical("list/viewport", f.clone())]
    ));
    (m, f)
}

// The speed curve.

#[test]
fn nothing_scrolls_until_the_pointer_is_more_than_12_points_from_the_anchor() {
    // browser/scrolling: The pointer must move more than 12 px from the anchor before scrolling starts; speed reaches 32 px per 16 ms frame at 232 px.
    assert_eq!(speed(12.0, &S), 0.0);
    assert_eq!(speed(-12.0, &S), 0.0);
    assert!(speed(12.5, &S) > 0.0);
    assert!(speed(-12.5, &S) < 0.0);
}

#[test]
fn the_speed_reaches_32_points_a_16_ms_frame_at_232_points_and_no_more() {
    // browser/scrolling: The pointer must move more than 12 px from the anchor before scrolling starts; speed reaches 32 px per 16 ms frame at 232 px.
    let per_frame = |d: f32| speed(d, &S) * 0.016;
    assert!((per_frame(232.0) - 32.0).abs() < 1e-3);
    assert!((per_frame(600.0) - 32.0).abs() < 1e-3);
    assert!((per_frame(-232.0) + 32.0).abs() < 1e-3);
    assert!(per_frame(231.0) < 32.0);
}

#[test]
fn the_further_the_pointer_the_faster_it_scrolls() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    let a = speed(50.0, &S);
    let b = speed(120.0, &S);
    let c = speed(200.0, &S);
    assert!(0.0 < a && a < b && b < c);
}

// The machine.

#[test]
fn a_listing_that_fits_starts_no_autoscroll() {
    // browser/scrolling: Middle-clicking a listing that fits its viewport starts no autoscroll.
    let mut m = Machine::default();
    assert!(!start(
        &mut m,
        vec![Target::vertical(
            "list/viewport",
            feed([0.0, 0.0], [0.0, 0.0])
        )]
    ));
    assert!(!m.active());
}

#[test]
fn only_one_autoscroll_runs_at_a_time() {
    // No bullet of its own: Strata's design holds one autoscroll a window
    // (`browser/scrolling`, Design), which is what lets any press end it.
    let (mut m, f) = running(0.0);
    assert!(!start(&mut m, vec![Target::vertical("other/viewport", f)]));
    assert_eq!(m.owner(), Some("list"));
}

#[test]
fn a_still_pointer_in_the_dead_zone_asks_for_nothing() {
    // browser/scrolling: The pointer must move more than 12 px from the anchor before scrolling starts; speed reaches 32 px per 16 ms frame at 232 px.
    let (mut m, _) = running(0.0);
    assert!(!m.track(Point::new(400.0, 310.0)));
    let mut out = Rec::default();
    assert!(!m.step(0.0, true, &mut out));
    assert!(out.0.is_empty(), "{:?}", out.0);
}

#[test]
fn a_pointer_below_the_anchor_glides_the_view_down_and_asks_for_the_next_frame() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    let (mut m, _) = running(1000.0);
    assert!(
        m.track(Point::new(400.0, 450.0)),
        "a sleeping machine wakes"
    );
    let mut out = Rec::default();
    m.step(0.0, true, &mut out);
    match &out.0[..] {
        [Req::Glide(path, _, y), Req::Wake] => {
            assert_eq!(path, "list/viewport");
            assert!(*y > 1000.0);
        }
        other => panic!("{other:?}"),
    }
    // Moving already: a further move needs no wake of its own.
    assert!(!m.track(Point::new(400.0, 460.0)));
}

#[test]
fn a_pointer_above_the_anchor_scrolls_up() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    let (mut m, _) = running(1000.0);
    m.track(Point::new(400.0, 150.0));
    let mut out = Rec::default();
    m.step(0.0, true, &mut out);
    assert!(matches!(out.0[0], Req::Glide(_, _, y) if y < 1000.0));
}

#[test]
fn with_animations_off_each_frame_jumps_one_frames_travel() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    let (mut m, _) = running(1000.0);
    m.track(Point::new(400.0, 300.0 + 232.0));
    let mut out = Rec::default();
    m.step(0.0, false, &mut out);
    match out.0[0] {
        // 2,000 pt/s for the assumed 1/60 s frame.
        Req::Jump(_, _, y) => assert!((y - (1000.0 + 2000.0 / 60.0)).abs() < 0.01, "{y}"),
        ref other => panic!("{other:?}"),
    }
}

#[test]
fn a_view_at_its_end_asks_nothing_further_that_way() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    let (mut m, _) = running(10_000.0);
    assert!(
        !m.track(Point::new(400.0, 500.0)),
        "nowhere to go: stays asleep"
    );
    let mut out = Rec::default();
    m.step(0.0, true, &mut out);
    assert!(out.0.is_empty());
    // Back up: it can go, so it wakes.
    assert!(m.track(Point::new(400.0, 100.0)));
}

#[test]
fn back_into_the_dead_zone_the_view_stops_where_it_is() {
    // browser/scrolling: The pointer must move more than 12 px from the anchor before scrolling starts; speed reaches 32 px per 16 ms frame at 232 px.
    let (mut m, f) = running(1000.0);
    m.track(Point::new(400.0, 500.0));
    m.step(0.0, true, &mut Rec::default());
    let mut p = f.get();
    p.offset[1] = 1040.0;
    f.set(p);
    m.track(Point::new(400.0, 305.0));
    let mut out = Rec::default();
    m.step(1.0 / 60.0, true, &mut out);
    assert_eq!(out.0, vec![Req::Jump("list/viewport".into(), 0.0, 1040.0)]);
}

#[test]
fn both_axes_move_on_a_viewport_that_scrolls_both_ways() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    let f = feed([500.0, 500.0], [2000.0, 2000.0]);
    let mut m = Machine::default();
    assert!(start(&mut m, vec![Target::both("grid", f)]));
    m.track(Point::new(250.0, 450.0));
    let mut out = Rec::default();
    m.step(0.0, true, &mut out);
    assert!(matches!(out.0[0], Req::Glide(_, x, y) if x < 500.0 && y > 500.0));
}

#[test]
fn a_sideways_only_viewport_moves_on_x_alone() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    let f = feed([100.0, 0.0], [900.0, 0.0]);
    let mut m = Machine::default();
    assert!(start(&mut m, vec![Target::horizontal("strip", f)]));
    m.track(Point::new(600.0, 600.0));
    let mut out = Rec::default();
    m.step(0.0, true, &mut out);
    assert!(matches!(out.0[0], Req::Glide(_, x, y) if x > 100.0 && y == 0.0));
}

#[test]
fn a_target_that_stops_is_stopped_once_while_another_moves_on() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    // (Review: with two targets, one at its end was jumped on every frame the other moved.)
    let down = feed([0.0, 0.0], [0.0, 10_000.0]);
    let side = feed([0.0, 0.0], [900.0, 0.0]);
    let mut m = Machine::default();
    assert!(start(
        &mut m,
        vec![
            Target::vertical("list/viewport", down.clone()),
            Target::horizontal("list-scroll", side.clone()),
        ]
    ));
    m.track(Point::new(700.0, 600.0));
    let mut rec = Rec::default();
    m.step(0.0, true, &mut rec);
    // The sideways scroll reaches its end; the vertical one goes on.
    side.set(feed([900.0, 0.0], [900.0, 0.0]).get());
    let jumps = |rec: &Rec| {
        rec.0
            .iter()
            .filter(|q| matches!(q, Req::Jump(p, ..) if p == "list-scroll"))
            .count()
    };
    rec.0.clear();
    m.step(1.0 / 60.0, true, &mut rec);
    assert_eq!(jumps(&rec), 1, "stopped as it reached its end: {:?}", rec.0);
    for i in 2..6 {
        rec.0.clear();
        m.step(f64::from(i) / 60.0, true, &mut rec);
        assert_eq!(jumps(&rec), 0, "and left alone after: {:?}", rec.0);
        assert!(
            rec.0
                .iter()
                .any(|q| matches!(q, Req::Glide(p, ..) if p == "list/viewport"))
        );
    }
}

#[test]
fn a_stop_leaves_the_view_where_it_is() {
    // browser/scrolling: Escape, or another click with any button anywhere in the window, stops autoscroll.
    let (mut m, f) = running(1000.0);
    m.track(Point::new(400.0, 500.0));
    m.step(0.0, true, &mut Rec::default());
    let mut p = f.get();
    p.offset[1] = 1020.0;
    f.set(p);
    let mut out = Rec::default();
    assert!(m.stop_now(&mut out));
    assert_eq!(out.0, vec![Req::Jump("list/viewport".into(), 0.0, 1020.0)]);
    assert!(!m.active());
    let mut out = Rec::default();
    assert!(!m.step(0.1, true, &mut out));
    assert!(out.0.is_empty(), "nothing after the stop");
}

#[test]
fn an_escape_stops_on_the_next_frame() {
    // browser/scrolling: Escape, or another click with any button anywhere in the window, stops autoscroll.
    let (mut m, _) = running(1000.0);
    m.track(Point::new(400.0, 500.0));
    m.step(0.0, true, &mut Rec::default());
    m.request_stop();
    assert!(m.active(), "stopping, until the frame's transaction");
    let mut out = Rec::default();
    assert!(
        m.step(1.0 / 60.0, true, &mut out),
        "the host draws the shield away"
    );
    assert!(matches!(out.0[..], [Req::Jump(..)]));
    assert!(!m.active());
}

#[test]
fn an_escape_in_the_dead_zone_stops_at_once() {
    // browser/scrolling: Escape, or another click with any button anywhere in the window, stops autoscroll.
    // (Review round 1: with nothing moving there is no glide to stop, so no frame need pass.)
    let (mut m, _) = running(1000.0);
    m.request_stop();
    assert!(!m.active(), "idle, not stopping");
    let mut out = Rec::default();
    assert!(!m.step(0.0, true, &mut out));
    assert!(out.0.is_empty());
}

#[test]
fn a_listing_that_goes_stops_only_its_own_autoscroll() {
    // browser/scrolling: Navigating away from, hiding, or closing a listing while it autoscrolls stops the autoscroll and hides the anchor.
    let (mut m, _) = running(0.0);
    assert!(!m.stop_gone(Some("another")));
    assert!(m.active());
    assert!(m.stop_gone(Some("list")));
    assert!(!m.active());
}

// The collection decorator.

struct Inner(Rc<std::cell::RefCell<Vec<PointerButton>>>);

impl PointerController for Inner {
    fn pointer(&mut self, input: &PointerInput, _cx: &mut Ctx<'_>) -> Flow {
        if input.phase == PointerPhase::Down {
            self.0.borrow_mut().push(input.button);
        }
        Flow::Handled
    }
}

struct Rig {
    source: ListSource,
    geometry: UniformRows,
    selection: crate::collection::selection::Selection,
    heard: Rc<std::cell::RefCell<Vec<PointerButton>>>,
    auto: Autoscroll,
    pointer: AutoscrollPointer,
}

impl Rig {
    fn new(len: usize) -> Self {
        Self::with(len, None)
    }

    /// `len` rows, and autoscroll also moving `sideways` (C4's sideways scroll).
    fn with(len: usize, sideways: Option<Target>) -> Self {
        let entries = (0..len as u64)
            .map(|i| {
                ListEntry::new(
                    EntryId::new(i + 1),
                    format!("f{i}"),
                    EntryKind::File,
                    "file",
                )
            })
            .collect();
        let heard = Rc::new(std::cell::RefCell::new(Vec::new()));
        let auto = Autoscroll::new();
        let content = len as f32 * 28.0;
        let f = feed([0.0, 0.0], [0.0, (content - 600.0).max(0.0)]);
        let mut pointer = AutoscrollPointer::new(
            Some(Box::new(Inner(heard.clone()))),
            auto.clone(),
            "list",
            f,
        );
        if let Some(t) = sideways {
            pointer = pointer.sideways(t);
        }
        Self {
            source: ListSource::new(entries),
            geometry: UniformRows::new(len, 28.0, 800.0).viewport_height(600.0),
            selection: crate::collection::selection::Selection::new(),
            heard,
            auto,
            pointer,
        }
    }

    /// A press with `button` at content point (`x`, `y`): the effects it asked for.
    fn press(&mut self, button: PointerButton, x: f32, y: f32) -> (Flow, Vec<Effect>) {
        let len = self.source.len();
        let index = (y / 28.0) as usize;
        let hit = Hit {
            index: (index < len).then_some(index),
            region: Region::Gutter,
        };
        let mut input = PointerInput::new(PointerPhase::Down, button, Point::new(x, y), hit);
        input.window = Point::new(x, y + 40.0);
        let mut settings = Settings::default();
        settings.insert(S);
        let mut cx = Ctx::new(
            &self.source,
            &self.geometry,
            Rect::new(0.0, 0.0, 800.0, 600.0),
            0.0,
            &mut self.selection,
        )
        .window_origin(Point::new(0.0, 40.0))
        .with_settings(Rc::new(settings));
        let flow = self.pointer.pointer(&input, &mut cx);
        let effects = cx.effects().to_vec();
        (flow, effects)
    }
}

use crate::contract::EntrySource as _;

#[test]
fn a_middle_press_on_empty_space_starts_autoscroll_at_the_press() {
    // browser/scrolling: Middle-clicking the empty background or gutter of a scrollable Columns, Icons, or List listing starts autoscroll. A round anchor marks the press point.
    let mut r = Rig::new(10);
    let (flow, _) = r.press(PointerButton::Middle, 400.0, 500.0);
    assert_eq!(flow, Flow::Handled);
    // Ten rows fit: nothing to scroll, so nothing started.
    assert!(!r.auto.active());
    assert_eq!(*r.heard.borrow(), vec![PointerButton::Middle]);

    let mut r = Rig::new(1000);
    // Content 28,000 tall: a press beside a row in its margin is background.
    let (flow, _) = r.press(PointerButton::Middle, 2.0, 300.0);
    assert_eq!(flow, Flow::Handled);
    assert!(r.auto.active());
    let (anchor, viewport) = r.auto.shown().expect("running");
    assert_eq!(
        anchor,
        Point::new(2.0, 340.0),
        "at the press, in window points"
    );
    assert_eq!(viewport, Rect::new(0.0, 40.0, 800.0, 600.0));
    assert!(r.heard.borrow().is_empty(), "the press goes no further");
}

#[test]
fn a_middle_press_on_an_entry_row_goes_to_the_row() {
    // browser/scrolling: Middle-clicking an entry row starts no autoscroll; the row's open-in-new-tab or reveal action runs instead.
    let mut r = Rig::new(1000);
    r.press(PointerButton::Middle, 300.0, 300.0);
    assert!(!r.auto.active());
    assert_eq!(*r.heard.borrow(), vec![PointerButton::Middle]);
}

#[test]
fn a_press_without_a_shield_stops_autoscroll_and_goes_no_further() {
    // browser/scrolling: The click that stops autoscroll does not also select, open, or start a marquee.
    let mut r = Rig::new(1000);
    r.press(PointerButton::Middle, 2.0, 300.0);
    let (flow, effects) = r.press(PointerButton::Primary, 300.0, 100.0);
    assert_eq!(flow, Flow::Handled);
    assert!(!r.auto.active());
    assert!(r.heard.borrow().is_empty());
    // It doesn't scroll its own viewport to stop (the autoscroll may be another listing's);
    // the next drive stops every target, with the transaction it has.
    assert!(!effects.iter().any(|e| matches!(e, Effect::ScrollTo { .. })));
}

#[test]
fn a_press_without_a_shield_stops_every_moving_target_on_the_next_frame() {
    // browser/scrolling: Escape, or another click with any button anywhere in the window, stops autoscroll.
    let side = feed([0.0, 0.0], [900.0, 0.0]);
    let mut r = Rig::with(1000, Some(Target::horizontal("list-scroll", side)));
    r.press(PointerButton::Middle, 2.0, 300.0);
    r.auto.track(Point::new(300.0, 640.0));
    let mut rec = Rec::default();
    r.auto.0.borrow_mut().step(0.0, true, &mut rec);
    rec.0.clear();
    r.press(PointerButton::Primary, 300.0, 100.0);
    assert!(r.auto.active(), "stopping, for the next frame");
    r.auto.0.borrow_mut().step(1.0 / 60.0, true, &mut rec);
    assert!(!r.auto.active());
    assert_eq!(
        rec.0,
        vec![
            Req::Jump("list/viewport".into(), 0.0, 0.0),
            Req::Jump("list-scroll".into(), 0.0, 0.0),
        ]
    );
}

#[test]
fn a_middle_press_moves_the_sideways_scroll_around_the_collection_too() {
    // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
    let side = feed([0.0, 0.0], [900.0, 0.0]);
    // Ten rows fit down; only the sideways scroll overflows, and that is enough to start.
    let mut r = Rig::with(10, Some(Target::horizontal("list-scroll", side)));
    r.press(PointerButton::Middle, 2.0, 100.0);
    assert!(r.auto.active());
    r.auto.track(Point::new(300.0, 640.0));
    let mut rec = Rec::default();
    r.auto.0.borrow_mut().step(0.0, true, &mut rec);
    let glides: Vec<_> = rec
        .0
        .iter()
        .filter_map(|q| match q {
            Req::Glide(path, x, y) => Some((path.as_str(), *x > 0.0, *y > 0.0)),
            _ => None,
        })
        .collect();
    // Down can't move (it fits); sideways glides right.
    assert_eq!(glides, vec![("list-scroll", true, false)]);
}

#[test]
fn a_middle_press_on_a_rows_margin_in_a_listing_that_fits_does_nothing() {
    // browser/scrolling: Middle-clicking a listing that fits its viewport starts no autoscroll.
    // (Strata's margin is background: the row's open-in-new-tab doesn't run either.)
    let mut r = Rig::new(10);
    let (flow, _) = r.press(PointerButton::Middle, 2.0, 100.0);
    assert_eq!(flow, Flow::Handled);
    assert!(!r.auto.active());
    assert!(r.heard.borrow().is_empty(), "the row didn't hear it");
    // The row itself still opens it.
    r.press(PointerButton::Middle, 300.0, 100.0);
    assert_eq!(*r.heard.borrow(), vec![PointerButton::Middle]);
}

#[test]
fn a_folder_change_stops_the_collections_autoscroll() {
    // browser/scrolling: Navigating away from, hiding, or closing a listing while it autoscrolls stops the autoscroll and hides the anchor.
    let mut r = Rig::new(1000);
    r.press(PointerButton::Middle, 2.0, 300.0);
    r.pointer.reset();
    assert!(!r.auto.active());
    assert!(r.auto.shown().is_none());
}
