//! The divider between the files and the preview: a 1 point line, and a 6 point grip inside
//! the preview's leading edge that moves it. The line and grip are one hover group, so the
//! pointer crossing between them keeps the caption. The files' own resize edges (C2's column
//! edges) sit outside it.
//!
//! - Hovering lights the line and tints the grip; resting 400 ms shows the caption.
//! - A pointer drag sets the manual width and shows a dashed outline from the divider to the
//!   trailing edge, captioned, which fades on release. Pressing hides the hover caption.
//! - Focused (Strata's F8), Left widens it a step and Right narrows it (the divider moves the
//!   way the arrow points), Up and Down widen and narrow it as a slider's do, Page Up and Page
//!   Down move ten steps, and Home and End go to the narrowest and widest. No outline shows.
//! - Its accessible value is what it sets: the width shown in Icons and List, and in Columns
//!   the session minimum (the manual width, else the width shown). Assistive increment and
//!   decrement widen and narrow it.

use std::time::Duration;

use rune::{Cursor, Drag, El, Key, KeyEvent, Phase, Transition, Tx, col, el, text};
use strata_kit::look::Look;

use super::{DrawerState, Shared, Solved};

/// The grip's key: the separator assistive technology reads, and where
/// [`DrawerState::focus_divider`] sends focus.
pub(super) const GRIP_KEY: &str = "preview-grip";

/// How long the pointer rests on the divider before its caption shows.
const CAPTION_DELAY: Duration = Duration::from_millis(400);

/// What the divider sets and reads as its value: in Columns the session minimum (the manual
/// width, else the width shown), elsewhere the width shown.
fn value(st: &DrawerState, solved: Solved, columns: bool) -> f32 {
    match st.manual_width() {
        Some(manual) if columns => manual,
        _ => solved.width,
    }
}

fn set<S: 'static>(shared: &Shared<S>, s: &mut S, solved: Solved, width: f32) {
    let w = width.clamp(solved.min, solved.max);
    (shared.access)(s).set_manual_width(&shared.rules, w);
}

pub(super) fn divider<S: 'static>(
    shared: &Shared<S>,
    st: &DrawerState,
    solved: Solved,
    columns: bool,
    look: &Look,
) -> El {
    let k = &look.kit;
    let rules = shared.rules;
    let now = value(st, solved, columns);
    let step = rules.step;

    let drag = shared.clone();
    let keys = shared.clone();
    let steps = shared.clone();
    let hover = shared.clone();
    let limits = shared.clone();
    // Keyed so a script can rest the pointer on either part.
    let line = el()
        .key("preview-divider-line")
        .a11y_hidden()
        .w(rules.line)
        .h_full()
        .fill(k.c.border_base)
        .child(
            el().key("preview-divider-lit")
                .w_full()
                .h_full()
                .fill(k.c.border_focus)
                // Lit by its own hover, or by the host's (a column's edge beside it, C2).
                .group_opacity(if st.lit { 1.0 } else { 0.0 }, 1.0),
        );
    let grip = el()
        .key("preview-divider-strip")
        .a11y_hidden()
        .w(rules.grip)
        .h_full()
        .fill(k.c.hover)
        .group_opacity(0.0, 1.0);
    col()
        .key(GRIP_KEY)
        .role("separator")
        .label(shared.labels.divider.clone())
        .range_value(f64::from(now), f64::from(solved.min), f64::from(solved.max))
        .range_step(f64::from(step))
        // The width in whole points as text too: what a reader says, and what a script checks.
        .a11y_value(format!("{now:.0}"))
        .focusable(true)
        .tab_index(-1)
        .w(rules.line + rules.grip)
        .h_full()
        .pin(Some(0.0), Some(0.0), None, Some(0.0))
        .raise()
        .hover_group()
        .cursor(Cursor::ResizeColumn)
        .transition(Transition::none())
        .child(rune::row().w_full().h_full().child(line).child(grip))
        .hover_delays(CAPTION_DELAY, Duration::ZERO)
        .on_hover_change(move |s: &mut S, inside: bool, _tx: &mut Tx| {
            (hover.access)(s).hint = inside;
        })
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| match d.phase {
            Phase::Down => {
                d.data = now;
                let st = (drag.access)(s);
                st.drag_from = st.manual_width();
                st.dragging = true;
                st.hint = false;
            }
            Phase::Move => {
                tx.instant();
                // The divider sits on the preview's leading edge: dragging left widens it.
                set(&drag, s, solved, d.data - d.dx);
            }
            Phase::Up => {
                let st = (drag.access)(s);
                if d.cancelled() {
                    // Escape, or the pointer taken away: back to where it started, as
                    // GtkPaned does.
                    st.cancel_drag();
                } else {
                    st.dragging = false;
                    st.drag_from = None;
                }
            }
        })
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let m = ev.mods;
            if m.shift || m.ctrl || m.alt || m.cmd || m.logo {
                return false;
            }
            let delta = match ev.key {
                Key::Left => step,
                Key::Right => -step,
                Key::PageUp => 10.0 * step,
                Key::PageDown => -10.0 * step,
                _ => return false,
            };
            tx.instant();
            set(&keys, s, solved, now + delta);
            true
        })
        // Up and Down, and assistive increment and decrement, reach here: Up widens.
        .on_step(move |s: &mut S, delta: i32, tx: &mut Tx| {
            tx.instant();
            set(&steps, s, solved, now + delta as f32 * step);
        })
        // Home and End: the narrowest and the widest.
        .on_step_limit(move |s: &mut S, widest: bool, tx: &mut Tx| {
            tx.instant();
            set(
                &limits,
                s,
                solved,
                if widest { solved.max } else { solved.min },
            );
        })
}

/// What floats over the split while the divider is in use: the hover caption, and the
/// minimum outline during a pointer drag.
pub(super) fn overlays<S: 'static>(
    shared: &Shared<S>,
    st: &DrawerState,
    solved: Solved,
    available: f32,
    slot_width: f32,
    columns: bool,
    look: &Look,
) -> Vec<El> {
    let k = &look.kit;
    let caption = |key: &'static str| {
        text(shared.labels.divider.clone())
            .key(key)
            .font(k.ty.small)
            .color(k.c.text_base)
            .px(look.space.sm)
            .py(look.space.xxs)
            .radius(look.round(k.radius.sm))
            .fill(k.c.surface_float)
            .border(1.0, k.c.border_muted)
    };
    let mut out = Vec::new();
    if st.hint && !st.dragging {
        let x = (available - slot_width + shared.rules.line + shared.rules.grip + look.space.xs)
            .max(0.0);
        out.push(
            caption("preview-divider-caption")
                .role("tooltip")
                .absolute(x, look.space.sm)
                .raise(),
        );
    }
    if st.dragging {
        let width = value(st, solved, columns).min(available);
        out.push(
            col()
                .key("preview-minimum-outline")
                .role("status")
                .label(shared.labels.divider.clone())
                .absolute((available - width).max(0.0), 0.0)
                .w(width)
                .h_full()
                .border(1.0, k.c.border_strong)
                .border_dash(4.0, 4.0)
                .padding(look.space.sm)
                .raise()
                // It fades on release, unless animations are off.
                .transition(if shared.animations {
                    Transition::opacity()
                } else {
                    Transition::none()
                })
                .child(caption("preview-minimum-outline-caption").a11y_hidden()),
        );
    }
    out
}
