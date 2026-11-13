// SPDX-License-Identifier: MIT

//! Return presses a focused button, as it does on GTK, where a button takes Return and Space
//! alike. Rune follows AppKit: Return goes to the window's default button and presses only a
//! focused row, link or `focus_highlight` element, and Space presses a focused control
//! (`takes_return` in Rune's `runtime/input/focus.rs`, and its README's key routing). Rune
//! packet entry 103 (`docs/rune/upstream/rune/103-return-presses-a-button.md`, finding
//! lgse/strata#2310) asks for a way to opt a button in; until then each button of this
//! interface that has the keyboard turns a plain Return into the Space that presses it, as C15's
//! dialogs do (`strata_components::dialog`'s key routing).
//!
//! Rune's `Cx::web_focus` would press every focused `on_press` element on Return, rows and
//! links' neighbours included, and change which elements are Tab stops: too broad for this.

use rune::{El, Key, KeyEvent, Mods, Tx};

/// The button `button`, pressed by a plain Return while it has the keyboard; a held Return
/// presses it once.
pub(crate) fn on_return(button: El) -> El {
    button.on_key(|_: &mut crate::App, event: &KeyEvent, tx: &mut Tx| pressed(event, tx))
}

/// What a button's key handler does with `event`: true when it took it.
pub(crate) fn pressed(event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.key != Key::Enter || event.mods != Mods::default() {
        return false;
    }
    if !tx.key_repeat() {
        tx.send_key(KeyEvent {
            key: Key::Text(" ".to_owned()),
            mods: Mods::default(),
        });
    }
    true
}

#[cfg(test)]
mod tests;
