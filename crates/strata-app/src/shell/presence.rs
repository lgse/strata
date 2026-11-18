// SPDX-License-Identifier: MIT

//! The window's presence, which each tab's browser rechecks its folders by
//! (`Browser::set_freshness`): `Hidden` while the window is off screen, `Focused` while it has
//! the keyboard, `Visible` otherwise, as `presence_of` in `src/ui/presence.rs` reads GTK's
//! window. GTK follows the window per tab (`presence::follow` in
//! `src/ui/window/composition/tabs.rs`); here every tab's browser, shown or parked, follows it
//! from the window's focus and visibility hooks.
//!
//! Rune reports a window going off screen (`App::on_visibility`: hidden, minimized or fully
//! covered on macOS and X11) but on Wayland neither covering nor minimizing, so a Wayland window
//! that is minimized stays `Visible`, where GTK's reads `MINIMIZED` from the toplevel state.

use std::rc::Rc;

use rune::Tx;
use strata_fs::app::{Browser, FreshnessInput, Presence};

use super::Window;
use crate::App;

/// The presence `window` gives its browsers.
pub(super) fn of(window: &Window) -> Presence {
    match (window.on_screen, window.active) {
        (false, _) => Presence::Hidden,
        (true, true) => Presence::Focused,
        (true, false) => Presence::Visible,
    }
}

/// Gives `browser` the presence, and keeps the rest of what it rechecks by.
pub(super) fn give(browser: &Rc<Browser>, presence: Presence) {
    let current = browser.freshness_input();
    if current.presence != presence {
        browser.set_freshness(FreshnessInput {
            presence,
            ..current
        });
    }
}

/// Every tab's browser follows the window's presence as it is now.
pub(super) fn follow(app: &App) {
    let presence = of(&app.window);
    give(&app.browser, presence);
    app.window
        .tabs
        .for_each_parked(|browser, _| give(browser, presence));
}

/// Rune's visibility hook: the window went off screen, or came back.
pub(crate) fn window_shown(app: &mut App, shown: bool, _: &mut Tx) {
    app.window.on_screen = shown;
    follow(app);
}

#[cfg(test)]
mod tests;
