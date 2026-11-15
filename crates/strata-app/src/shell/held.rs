// SPDX-License-Identifier: MIT

//! The modifier keys held down in the window, for a part that shows more while one is held:
//! GTK's tab strip numbers its tabs while Ctrl+Shift is down (`handle_key` and `install_keys`
//! in `src/ui/window/composition/tabs.rs`). The window keeps one set; a part reads it at build
//! time with [`held_modifiers`], never as a command, so a modal layer that cuts a stage's
//! commands does not hide what it shows.
//!
//! The windowed host hears them through Rune's `App::on_modifiers`, which calls [`modifiers`]
//! on every change wherever focus is and once with none held when the window loses focus. At
//! the pinned rev its reference is the method's doc comment (`crates/rune/src/app.rs`, lines
//! 270 to 283) and the changelog entry `changelog/unreleased/2026-11-08-held-modifiers.md`;
//! Rune's README has no section for it yet.
//!
//! The offscreen host cannot register the hook: a `Runtime` it builds itself keeps
//! `on_mods` crate-private (Rune packet entry 107,
//! `docs/rune/upstream/rune/107-runtime-on-modifiers.md`). It calls [`modifiers`] itself
//! wherever it feeds Rune the held modifiers (`Offscreen::modifiers`), and the root's
//! `on_window_focus` clears them when the window blurs. `Offscreen::modifiers` updates them as
//! it feeds them, ahead of the key events queued beside them, so a key handler may see the
//! modifiers held after the key; a part reads them while it builds, where both hosts agree.

use rune::{Mods, Tx};

use crate::App;

/// Rune's `App::on_modifiers` hook: keeps `mods` as the window's held modifiers. Unchanged,
/// the view reads nothing new.
pub(crate) fn modifiers(app: &mut App, mods: Mods, tx: &mut Tx) {
    if app.window.held == mods {
        tx.live_only();
        return;
    }
    app.window.held = mods;
}

/// The modifiers held down in the window as of the last change heard, none while the window
/// has no keyboard focus.
pub(crate) fn held_modifiers(app: &App) -> Mods {
    app.window.held
}
