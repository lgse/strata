// SPDX-License-Identifier: MIT

//! What the dialog does with a key, as plain decisions the panel's key handler acts on.

use rune::{Key, KeyEvent, Mods, TextEdit, Tx};
use rune_kit::TextArea;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// Leave the key to Rune: the default button, focus navigation, the window.
    Pass,
    /// Press the focused control: Enter does what Space does.
    Press,
    /// Move focus to the footer button at this index.
    Focus(usize),
    /// Take the key and do nothing.
    Consume,
}

pub(crate) fn route(
    ev: &KeyEvent,
    repeat: bool,
    focused: Option<&str>,
    panel: &str,
    footer: &[String],
) -> Route {
    if ev.mods != Mods::default() {
        return Route::Pass;
    }
    match ev.key {
        Key::Enter => match focused {
            // Nothing in the dialog has focus: Return goes to the primary, the default button.
            None => Route::Pass,
            Some(f) if f == panel => Route::Pass,
            // A held Return presses once.
            Some(_) if repeat => Route::Consume,
            Some(_) => Route::Press,
        },
        Key::Left | Key::Right => {
            let Some(i) = focused.and_then(|f| footer.iter().position(|k| k == f)) else {
                return Route::Pass;
            };
            match ev.key {
                Key::Left if i == 0 => Route::Consume,
                Key::Left => Route::Focus(i - 1),
                _ if i + 1 == footer.len() => Route::Consume,
                _ => Route::Focus(i + 1),
            }
        }
        _ => Route::Pass,
    }
}

pub fn newline_on_enter<S: 'static>() -> impl Fn(&mut S, &KeyEvent, &mut Tx) -> bool + 'static {
    |_s, ev, tx| {
        if ev.key != Key::Enter || ev.mods != Mods::default() {
            return false;
        }
        tx.send_key(KeyEvent {
            key: Key::Enter,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        });
        true
    }
}

pub fn text_area<S: 'static>(
    edit: &TextEdit,
    access: impl for<'a> Fn(&'a mut S) -> &'a mut TextEdit + 'static,
) -> TextArea {
    TextArea::new(edit)
        .on_edit(access)
        .on_key(newline_on_enter::<S>())
}
