// SPDX-License-Identifier: MIT

//! [`OutsideWheel`] on a Rune runtime: what a tick outside a panel closes and scrolls.

use rune::{Cx, El, Env, Mods, Runtime, ScrollPos, WheelEvent, col, el};

use super::OutsideWheel;
use crate::scroll::{ScrollFeed, Target};

const W: f32 = 600.0;
const H: f32 = 400.0;
const FRAME: f64 = 1.0 / 60.0;
/// The open panel's box, clear of the window's centre where the other ticks land.
const PANEL: [f32; 4] = [20.0, 20.0, 120.0, 100.0];

struct App {
    open: bool,
    feed: ScrollFeed,
}

/// A viewport that scrolls both ways under the whole window, and while `open` the outside
/// layer over it and a panel over that, which scrolls nothing and takes a press.
fn view(s: &App, _: &mut Cx<App>) -> El {
    let feed = s.feed.clone();
    let viewport = col()
        .key("listing")
        .size(W, H)
        .scroll_xy()
        .on_scroll_offset(move |p: ScrollPos| feed.set(p))
        .child(col().size(4.0 * W, 4.0 * H));
    let mut root = col().size(W, H).child(viewport);
    if s.open {
        let outside = OutsideWheel::new(|s: &mut App, _| s.open = false)
            .listings(|s: &App, _| vec![Target::both("listing", s.feed.clone())]);
        let [x, y, w, h] = PANEL;
        let panel = el()
            .key("panel")
            .absolute(x, y)
            .size(w, h)
            .on_press(|_: &mut App| {});
        root = root.child(outside.scrim("scrim")).child(panel);
    }
    root
}

fn runtime() -> Runtime<App> {
    let app = App {
        open: true,
        feed: ScrollFeed::new(),
    };
    let mut rt = Runtime::new(app, view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(FRAME);
    rt
}

fn tick(rt: &mut Runtime<App>, ev: WheelEvent) {
    tick_at(rt, ev, (W / 2.0, H / 2.0));
}

fn tick_at(rt: &mut Runtime<App>, ev: WheelEvent, (x, y): (f32, f32)) {
    rt.wheel_event(ev.at(x, y));
    for _ in 0..3 {
        rt.frame(FRAME);
    }
}

#[test]
fn a_tick_outside_closes_the_panel_and_moves_the_listing_by_its_delta() {
    // browser/scrolling: The tick that closes a panel also scrolls the listing under the pointer, and no other; over the sidebar or chrome it only closes the panel.
    let mut rt = runtime();
    tick(&mut rt, WheelEvent::new(0.0, -120.0).precise(false));
    assert!(!rt.state.open);
    assert_eq!(rt.state.feed.get().offset, [0.0, 120.0]);
}

#[test]
fn two_ticks_outside_in_one_frame_move_the_listing_by_both() {
    // browser/scrolling: That forwarded tick moves the listing by the step a wheel notch normally moves it.
    // Review round 1, SF2: the feed moves once a frame, so each tick builds on the ones before
    // it in the same frame.
    let mut rt = runtime();
    rt.wheel_event(
        WheelEvent::new(0.0, -120.0)
            .precise(false)
            .at(W / 2.0, H / 2.0),
    );
    tick(&mut rt, WheelEvent::new(0.0, -120.0).precise(false));
    assert!(!rt.state.open);
    assert_eq!(rt.state.feed.get().offset, [0.0, 240.0]);
}

#[test]
fn ticks_outside_in_one_frame_stop_at_the_listing_end() {
    // Two ticks past the end, then one back by half: the listing stands half way, not past
    // the end and still clamped there.
    let mut rt = runtime();
    let max = rt.state.feed.get().max[1];
    assert!(max > 0.0);
    for _ in 0..2 {
        rt.wheel_event(
            WheelEvent::new(0.0, -max)
                .precise(false)
                .at(W / 2.0, H / 2.0),
        );
    }
    tick(&mut rt, WheelEvent::new(0.0, max / 2.0).precise(false));
    assert_eq!(rt.state.feed.get().offset, [0.0, max / 2.0]);
}

#[test]
fn shift_turns_a_vertical_tick_outside_sideways() {
    // browser/scrolling: Shift+wheel outside an open panel closes it and scrolls the pointed listing horizontally.
    // (In process: a script's Shift + wheel doesn't reach Rune's turn, U-C24-7.)
    let mut rt = runtime();
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    tick(
        &mut rt,
        WheelEvent::new(0.0, -120.0).precise(false).mods(shift),
    );
    assert!(!rt.state.open);
    assert_eq!(rt.state.feed.get().offset, [120.0, 0.0]);
}

#[test]
fn a_shift_tick_the_window_host_turned_stays_sideways() {
    // Rune's window host turns a Shift + mouse-wheel tick before the scrim hears it; turning
    // it again would scroll the listing vertically.
    let mut rt = runtime();
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    tick(
        &mut rt,
        WheelEvent::new(-120.0, 0.0).precise(false).mods(shift),
    );
    assert!(!rt.state.open);
    assert_eq!(rt.state.feed.get().offset, [120.0, 0.0]);
}

#[test]
fn a_tick_over_the_panel_neither_closes_it_nor_scrolls_the_listing_under_it() {
    // browser/scrolling: A wheel over an open panel that doesn't scroll moves nothing. Rune
    // starts the wheel on the panel and chains it up the panel's ancestors only (#1137), so the
    // panel needs no wheel handler of its own (draft U-C24b-2, retired).
    let mut rt = runtime();
    let [x, y, w, h] = PANEL;
    tick_at(
        &mut rt,
        WheelEvent::new(0.0, -120.0).precise(false),
        (x + w / 2.0, y + h / 2.0),
    );
    assert!(rt.state.open);
    assert_eq!(rt.state.feed.get().offset, [0.0, 0.0]);
}
