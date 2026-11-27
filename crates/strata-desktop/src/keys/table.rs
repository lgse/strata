// SPDX-License-Identifier: MIT

use super::{Backend, Key, Modifiers};

macro_rules! declared {
    ($($name:ident = $keysym:literal,)*) => {
        &[$((stringify!($name), $keysym)),*]
    };
}

/// Every declared constant: its identifier and its keysym.
const DECLARED: &[(&str, u32)] = crate::key_constants!(declared);

const UNICODE: u32 = 0x0100_0000;

/// A constant's identifier is the key's name, with an underscore before a leading digit.
fn name_of(identifier: &'static str) -> &'static str {
    match identifier.strip_prefix('_') {
        Some(name) if name.starts_with(|first: char| first.is_ascii_digit()) => name,
        _ => identifier,
    }
}

const WRITTEN_AS: &[(Key, &str)] = &[
    (Key::dead_tilde, "dead_perispomeni"),
    (Key::Mode_switch, "Arabic_switch"),
    (Key::KP_Page_Down, "KP_Next"),
];

const KEYPAD: &[(Key, char)] = &[
    (Key::KP_Space, ' '),
    (Key::KP_Equal, '='),
    (Key::KP_Multiply, '*'),
    (Key::KP_Add, '+'),
    (Key::KP_Separator, ','),
    (Key::KP_Subtract, '-'),
    (Key::KP_Decimal, '.'),
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
