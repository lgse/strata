// SPDX-License-Identifier: MIT

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
        let delta = if sideways { ev.dx } else { ev.dy };
        let to = (probe.wheel_base() - delta).max(0.0);
        probe.wheel_jumped(to);
        tx.scroll_to_instant(scroll.clone(), to, 0.0);
        true
    })
}
