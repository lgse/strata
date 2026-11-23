// SPDX-License-Identifier: MIT

use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::Duration;

use rune::el;

use super::state::Change;
use super::{Shared, anchor_key};
use crate::collection::element::{Collection, Rest, RowCx};
use crate::contract::Rect;

pub(super) fn rows<S: 'static>(sh: &Rc<Shared<S>>, base: Collection<S>) -> Collection<S> {
    let (delays, report, version, part, view) =
        (sh.clone(), sh.clone(), sh.clone(), sh.clone(), sh.clone());
    let drawn = sh.clone();
    base.rest(
        Rest::new(
            move |s: &S, rcx: &RowCx<'_>| {
                let st = (delays.get)(s);
                if !rcx.entry.kind.is_folder_like() {
                    return None;
                }
                let b = delays.behavior;
                // Back from the card onto its folder, the peek stays at once.
                let enter = if st.is_anchor(rcx.scope, rcx.entry.id) {
                    Duration::ZERO
                } else {
                    b.open
                };
                Some((enter, b.close))
            },
            move |s: &mut S, mut r, tx| {
                if let Some(bounds) = report.bounds.as_ref().and_then(|f| f(s)) {
                    r.viewport = clip(r.viewport, bounds);
                }
                let change = (report.get_mut)(s).row_rest(&r);
                report.changed(s, change, tx);
            },
        )
        .on_view(move |s: &mut S, v| {
            let change = (view.get_mut)(s).view(&v);
            if change == Change::None {
                return false;
            }
            if let Some(f) = &view.on_change {
                f(s, change, &mut rune::Tx::default());
            }
            true
        })
        .part(
            move |s: &S, rcx: &RowCx<'_>| {
                let mut h = std::hash::DefaultHasher::new();
                anchor_dx(&version, s, rcx).map(f32::to_bits).hash(&mut h);
                h.finish()
            },
            move |s: &S, rcx: &RowCx<'_>| {
                let dx = anchor_dx(&part, s, rcx)?;
                let drawn = drawn.clone();
                Some(
                    el().key(anchor_key(rcx.scope))
                        .pin(Some(dx), Some(0.0), None, Some(0.0))
                        .w(0.0)
                        .on_drawn_layout_tx(move |s: &mut S, r, tx| {
                            let change = (drawn.get_mut)(s).anchor_drawn(r[0]);
                            drawn.changed(s, change, tx);
                        }),
                )
            },
        ),
    )
}

/// The part of `a` inside `b`; empty (no width) when they don't meet.
pub(super) fn clip(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let w = (a.x + a.w).min(b.x + b.w) - x;
    let h = (a.y + a.h).min(b.y + b.h) - y;
    Rect::new(x, y, w.max(0.0), h.max(0.0))
}

/// The anchor's offset in this row, when a peek is open from it.
fn anchor_dx<S>(sh: &Shared<S>, s: &S, rcx: &RowCx<'_>) -> Option<f32> {
    let st = (sh.get)(s);
    if !st.is_anchor(rcx.scope, rcx.entry.id) {
        return None;
    }
    st.placement().map(|p| p.dx)
}
