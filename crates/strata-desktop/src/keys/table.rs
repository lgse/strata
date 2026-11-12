// SPDX-License-Identifier: MIT

//! A key backend that needs no library. It answers for the keys Strata names in code and for
//! characters, and for nothing else:
//!
//! - **Names:** the declared constants of [`key_constants`](crate::key_constants), each read
//!   by its own name and written by it, but for the three keys GDK writes by another of
//!   their X names ([`WRITTEN_AS`]). Any other key has no name here, and any other name no
//!   key. `VoidSymbol`, which is no key, has no name and its name reads as none.
//! - **Characters:** a printable Latin-1 character is its own keysym, and any other character
//!   is its Unicode keysym, `0x0100_0000` plus the code point, as the X protocol defines
//!   both. The sixteen keypad keys that type a character type it ([`KEYPAD`]). No other
//!   named key types anything here.
//! - **Case:** the ASCII letters. Every other key is its own lower and upper case.
//! - **Accelerator strings:** not read and not written. See [`Table`].
//!
//! It is the table the `xkbcommon` backend will sit behind: that backend asks the library
//! only for what this table does not answer, so every key the catalog binds reads and writes
//! the same whatever library a user has installed.

use super::{Backend, Key, Modifiers};

macro_rules! declared {
    ($($name:ident = $keysym:literal,)*) => {
        &[$((stringify!($name), $keysym)),*]
    };
}

/// Every declared constant: its identifier and its keysym.
const DECLARED: &[(&str, u32)] = crate::key_constants!(declared);

/// The first Unicode keysym: a character that has no keysym of its own is this plus its
/// code point.
const UNICODE: u32 = 0x0100_0000;

/// A constant's identifier is the key's name, with an underscore before a leading digit.
fn name_of(identifier: &'static str) -> &'static str {
    match identifier.strip_prefix('_') {
        Some(name) if name.starts_with(|first: char| first.is_ascii_digit()) => name,
        _ => identifier,
    }
}

/// Keys GDK writes by another of their X names than the constant's. Both names read. Found
/// by running the GTK application's test of this table against GDK, which holds them.
const WRITTEN_AS: &[(Key, &str)] = &[
    (Key::dead_tilde, "dead_perispomeni"),
    (Key::Mode_switch, "Arabic_switch"),
    (Key::KP_Page_Down, "KP_Next"),
];

/// The keypad keys that type a character, which GDK answers too: a press of one reports
/// the keypad key, and its text is this.
const KEYPAD: &[(Key, char)] = &[
    (Key::KP_Space, ' '),
    (Key::KP_Equal, '='),
    (Key::KP_Multiply, '*'),
    (Key::KP_Add, '+'),
    (Key::KP_Subtract, '-'),
    (Key::KP_Divide, '/'),
    (Key::KP_0, '0'),
    (Key::KP_1, '1'),
    (Key::KP_2, '2'),
    (Key::KP_3, '3'),
    (Key::KP_4, '4'),
    (Key::KP_5, '5'),
    (Key::KP_6, '6'),
    (Key::KP_7, '7'),
    (Key::KP_8, '8'),
    (Key::KP_9, '9'),
];

fn printable_latin_1(code: u32) -> bool {
    matches!(code, 0x20..=0x7e | 0xa0..=0xff)
}

/// The backend over the declared keys.
///
/// It reads and writes no accelerator string: [`Backend::accelerator_parse`] answers `None`
/// for every string, so a saved keymap entry is reported as one that cannot be read and the
/// shipped keys apply, and [`Backend::accelerator_name`] panics, because a string it made up
/// would be saved over a user's binding. Both arrive with the backend that sits on this one.
pub struct Table;

impl Backend for Table {
    fn key_name(&self, key: Key) -> Option<String> {
        if key == Key::VoidSymbol {
            return None;
        }
        let other = WRITTEN_AS.iter().find(|(written, _)| *written == key);
        let own = DECLARED
            .iter()
            .find(|(_, keysym)| *keysym == key.keysym())
            .map(|(identifier, _)| name_of(identifier));
        other.map(|(_, name)| *name).or(own).map(str::to_owned)
    }

    fn key_from_name(&self, name: &str) -> Option<Key> {
        // As GDK's backend answers: the name of no key at all is not a key's name.
        if name == "VoidSymbol" {
            return None;
        }
        let other = WRITTEN_AS.iter().find(|(_, written)| *written == name);
        let own = DECLARED
            .iter()
            .find(|(identifier, _)| name_of(identifier) == name)
            .map(|(_, keysym)| Key::from_keysym(*keysym));
        other.map(|(key, _)| *key).or(own)
    }

    fn key_to_unicode(&self, key: Key) -> Option<char> {
        let keysym = key.keysym();
        if printable_latin_1(keysym) {
            return char::from_u32(keysym);
        }
        if let Some((_, character)) = KEYPAD.iter().find(|(keypad, _)| *keypad == key) {
            return Some(*character);
        }
        // The Unicode keysym of no character at all types nothing.
        keysym
            .checked_sub(UNICODE)
            .filter(|code| *code != 0)
            .and_then(char::from_u32)
    }

    fn key_from_unicode(&self, character: char) -> Key {
        let code = u32::from(character);
        Key::from_keysym(if printable_latin_1(code) {
            code
        } else {
            UNICODE + code
        })
    }

    fn key_to_lower(&self, key: Key) -> Key {
        match u8::try_from(key.keysym()) {
            Ok(letter) if letter.is_ascii_uppercase() => {
                Key::from_keysym(u32::from(letter.to_ascii_lowercase()))
            }
            _ => key,
        }
    }

    fn key_to_upper(&self, key: Key) -> Key {
        match u8::try_from(key.keysym()) {
            Ok(letter) if letter.is_ascii_lowercase() => {
                Key::from_keysym(u32::from(letter.to_ascii_uppercase()))
            }
            _ => key,
        }
    }

    fn accelerator_parse(&self, _accelerator: &str) -> Option<(Key, Modifiers)> {
        None
    }

    fn accelerator_name(&self, key: Key, modifiers: Modifiers) -> String {
        panic!(
            "the table key backend writes no accelerator strings, asked for {key:?} with \
             {modifiers:?}: writing one arrives with the xkbcommon backend"
        )
    }
}

#[cfg(test)]
mod tests;
