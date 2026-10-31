// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_window_is_hidden_unless_mapped_and_shown_and_focused_only_when_active() {
    let suspended = gdk::ToplevelState::SUSPENDED | gdk::ToplevelState::FOCUSED;
    let mut failures = Vec::new();
    for (mapped, active, state, expected) in [
        (false, true, gdk::ToplevelState::FOCUSED, Presence::Hidden),
        (true, true, suspended, Presence::Hidden),
        (true, false, gdk::ToplevelState::MINIMIZED, Presence::Hidden),
        (true, true, gdk::ToplevelState::FOCUSED, Presence::Focused),
        (true, false, gdk::ToplevelState::empty(), Presence::Visible),
    ] {
        let got = presence(mapped, active, state);
        if got != expected {
            failures.push(format!(
                "mapped {mapped}, active {active}, {state:?}: got {got:?}, expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
