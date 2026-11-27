// SPDX-License-Identifier: MIT

use rune::{Mods, Tx};

use crate::App;

pub(crate) fn modifiers(app: &mut App, mods: Mods, tx: &mut Tx) {
    if app.window.held == mods {
        tx.live_only();
        return;
    }
    app.window.held = mods;
    crate::dnd::modifiers_changed(app, tx);
}

pub(crate) fn held_modifiers(app: &App) -> Mods {
    app.window.held
}
