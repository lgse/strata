// SPDX-License-Identifier: MIT

use rune::{El, Key, KeyEvent, Mods, Tx};

pub(crate) fn on_return(key: impl Into<String>, button: El) -> El {
    taking_return(key, button)
        .on_key(|_: &mut crate::App, event: &KeyEvent, tx: &mut Tx| pressed(event, tx))
}

pub(crate) fn taking_return(key: impl Into<String>, button: El) -> El {
    let key = key.into();
    #[cfg(test)]
    tests::taking_return(&key);
    button.key(key)
}

pub(crate) fn on_return_within(key: &'static str, around: El) -> El {
    #[cfg(test)]
    tests::taking_return(key);
    around.on_key(move |_: &mut crate::App, event: &KeyEvent, tx: &mut Tx| {
        tx.focused() == Some(key) && pressed(event, tx)
    })
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
