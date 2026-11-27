// SPDX-License-Identifier: MIT

use super::{Key, KeyPress, MODIFIER_KEYS, Modifiers, SHORTCUT_MODIFIERS, alias, main_key};

/// What the press typed, for text and for telling a modifier key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    /// One character, as the layout typed it with the held modifiers.
    Character(char),
    Key(Key),
    Nothing,
}

/// What [`super::normalize`] derives from a GDK event, given directly.
#[derive(Clone, Copy, Debug)]
pub struct KeyPressParts {
    pub key: Key,
    pub modifiers: Modifiers,
    pub held: Modifiers,
    pub typed: Typed,
    pub digit: Option<u8>,
    /// Whether `key` came from another layout or from the US table.
    pub fallback: bool,
}

impl KeyPress {
    pub fn from_parts(parts: KeyPressParts) -> Self {
        let mut modifiers = parts.modifiers & SHORTCUT_MODIFIERS;
        let key = if parts.key == Key::ISO_Left_Tab {
            modifiers.insert(Modifiers::SHIFT_MASK);
            Key::Tab
        } else {
            main_key(parts.key.to_lower())
        };
        let typed = match parts.typed {
            Typed::Character(character) => Key::from_char(character),
            Typed::Key(key) => key,
            Typed::Nothing => Key::VoidSymbol,
        };
        Self {
            key,
            shortcut_modifiers: modifiers,
            state: parts.held & SHORTCUT_MODIFIERS,
            typed,
            digit: parts.digit.filter(|digit| *digit <= 9),
            fallback: parts.fallback,
            modifier: MODIFIER_KEYS.contains(&typed),
            aliasing: alias::Aliasing::Off,
            alias: None,
        }
    }

    pub fn unclaimed_alias(&self) -> Option<(Key, Modifiers)> {
        self.alias
            .is_some_and(|alias| !alias.typed)
            .then_some((self.key, self.shortcut_modifiers))
    }
}
