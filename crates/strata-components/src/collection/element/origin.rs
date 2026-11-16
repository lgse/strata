// SPDX-License-Identifier: MIT

//! External marquee origin routing ([`super::Collection::origin_drag`]): primary drags starting on
//! blank chrome outside the collection's viewport (such as the preview drawer).

use std::rc::Rc;

use rune::{Drag, Mods, Phase, PointerButton, Tx};

use super::Config;
use super::route::{Source, begin_marquee, dispatch};
use super::seam::Flow;
use crate::collection::pointer::PointerSettings;
use crate::contract::Point;

pub(super) fn origin_drag<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    (d, clears_on_press): (&mut Drag, bool),
    tx: &mut Tx,
) {
    if tx.button() != PointerButton::Primary {
        tx.live_only();
        return;
    }
    match d.phase {
        Phase::Down => origin_down(cfg, s, (d, clears_on_press), tx),
        Phase::Move => origin_move(cfg, s, d, tx),
        Phase::Up => origin_up(cfg, s, d, tx),
    }
}

fn is_plain_mods(mods: Mods) -> bool {
    !(mods.ctrl || mods.shift || mods.cmd)
}

fn marquee_active<S>(cfg: &Config<S>, s: &S) -> bool {
    (cfg.get)(s)
        .1
        .controllers
        .marquee
        .as_ref()
        .is_some_and(|m| m.active())
}

fn origin_down<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    (d, clears_on_press): (&Drag, bool),
    tx: &mut Tx,
) {
    let [wx, wy] = d.window_point();
    let mods = tx.mods();
    (cfg.get_mut)(s).1.origin_press = Some((Point::new(wx, wy), mods));
    if clears_on_press && is_plain_mods(mods) {
        dispatch(cfg, s, tx, Source::Pointer(None), |_, cx| {
            cx.apply(|sel, _| sel.clear());
            Flow::Handled
        });
    } else if !clears_on_press {
        tx.live_only();
    }
}

fn origin_move<S: 'static>(cfg: &Rc<Config<S>>, s: &mut S, d: &Drag, tx: &mut Tx) {
    if d.cancelled() {
        cancel_origin(cfg, s, tx);
        return;
    }
    let [wx, wy] = d.window_point();
    let window = Point::new(wx, wy);
    if let Some((start, mods)) = (cfg.get)(s).1.origin_press {
        maybe_start_marquee(cfg, s, (d, window), (start, mods, tx));
        return;
    }
    if !marquee_active(cfg, s) {
        tx.live_only();
        return;
    }
    let now_pt = (cfg.get)(s).1.content_point(window);
    let (_, shows) = dispatch(cfg, s, tx, Source::Pointer(Some(now_pt)), |c, cx| {
        if let Some(m) = c.marquee.as_mut() {
            m.moved(now_pt, cx);
        }
        Flow::Handled
    });
    if !shows {
        tx.live_only();
    }
}

fn maybe_start_marquee<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    (d, window): (&Drag, Point),
    (start, mods, tx): (Point, Mods, &mut Tx),
) {
    let threshold = cfg
        .settings
        .get::<PointerSettings>()
        .map_or(4.0, |p| p.drag_threshold);
    if d.dx.abs() <= threshold && d.dy.abs() <= threshold {
        tx.live_only();
        return;
    }
    (cfg.get_mut)(s).1.origin_press = None;
    let (anchor, now_pt) = {
        let st = (cfg.get)(s).1;
        (st.content_point(start), st.content_point(window))
    };
    begin_marquee(cfg, s, tx, anchor, mods, Some(now_pt));
}

fn cancel_origin<S: 'static>(cfg: &Rc<Config<S>>, s: &mut S, tx: &mut Tx) {
    (cfg.get_mut)(s).1.origin_press = None;
    if marquee_active(cfg, s) {
        dispatch(cfg, s, tx, Source::Pointer(None), |c, cx| {
            if let Some(m) = c.marquee.as_mut() {
                m.cancel(cx);
            }
            Flow::Handled
        });
    } else {
        tx.live_only();
    }
}

fn origin_up<S: 'static>(cfg: &Rc<Config<S>>, s: &mut S, d: &Drag, tx: &mut Tx) {
    (cfg.get_mut)(s).1.origin_press = None;
    if !marquee_active(cfg, s) {
        tx.live_only();
        return;
    }
    let cancelled = d.cancelled();
    dispatch(cfg, s, tx, Source::Pointer(None), |c, cx| {
        if let Some(m) = c.marquee.as_mut() {
            if cancelled {
                m.cancel(cx);
            } else {
                m.end(cx);
            }
        }
        Flow::Handled
    });
}
