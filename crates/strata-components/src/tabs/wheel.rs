//! A vertical wheel over the tabs scrolls the strip sideways (`browser/tabs`: "When the strip
//! is wider than the window, the wheel scrolls it horizontally").
//!
//! Rune sends a vertical wheel only to a vertical viewport, so over a sideways-only strip it
//! scrolls nothing (U-C6-5), and it has no way to read a viewport's offset or scroll one
//! relatively (U-C7-2). So this is a workaround, and fragile:
//!
//! - every tab takes the wheel through `on_wheel` (from `TabItem::decorate`), which Rune asks
//!   before the strip's viewport (probe 2 in `strata-spikes/c7-probes`); a wheel over the gap
//!   between two tabs reaches none of them and scrolls nothing;
//! - the offset is read from where hidden markers drew (`probe.rs`, with its caveats);
//! - a mostly sideways gesture is left to the viewport, which scrolls it itself, except in the
//!   frame a reveal's jump went out (`state.rs`), which it would undo;
//! - once a tab's handler has taken a delta, Rune hands it the rest of that gesture and drops
//!   what it declines, so it scrolls every later delta itself, sideways ones too, until a
//!   trackpad's gesture ends or the next begins. A mouse wheel's deltas carry no phase, so after
//!   one it keeps scrolling the tabs' sideways wheels itself; that scrolls as the viewport
//!   would. Deltas within one frame add up (`Probe::wheel_base`), since the markers report the
//!   offset once a frame;
//! - any wheel over a tab cancels a reveal still to land, so the user's scroll wins. A wheel over
//!   the gap between two tabs reaches no tab and doesn't.

use rune::{El, GesturePhase, Tx, WheelEvent};

use super::probe::Probe;

/// `tab`, scrolling the strip's viewport `scroll` sideways under a vertical wheel.
pub(super) fn sideways<S: 'static>(tab: El, probe: &Probe, scroll: String) -> El {
    let probe = probe.clone();
    tab.on_wheel(move |_: &mut S, ev: WheelEvent, tx: &mut Tx| {
        // The user's scroll wins over a reveal still to land (`state.rs`), whichever way.
        probe.note_wheel();
        if ev.phase == GesturePhase::Began {
            probe.set_owning(false);
        }
        let ends = ev.phase == GesturePhase::Ended;
        let sideways = ev.dy.abs() <= ev.dx.abs();
        // Once a tab's handler takes a delta, Rune hands it the rest of the gesture and drops
        // any it declines (`run_wheel`), so it scrolls them all, sideways ones too (review 3).
        // Otherwise a sideways gesture is the viewport's, unless a reveal's jump went out this
        // frame: the jump would land after it and undo it, so this wheel jumps instead, from
        // where the screen shows the strip, and lands last (review 2).
        let owning = probe.owning();
        if !owning && sideways && !probe.revealing() {
            return false;
        }
        if ev.dx == 0.0 && ev.dy == 0.0 {
            if ends {
                probe.set_owning(false);
            }
            return owning;
        }
        probe.set_owning(!ends);
        // A wheel turned down (content moving up, `dy` negative) shows what is to the right;
        // one moving content right (`dx` positive) shows what is to the left.
        let delta = if sideways { ev.dx } else { ev.dy };
        let to = (probe.wheel_base() - delta).max(0.0);
        probe.wheel_jumped(to);
        tx.scroll_to_instant(scroll.clone(), to, 0.0);
        true
    })
}
