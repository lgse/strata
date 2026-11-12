// SPDX-License-Identifier: MIT

//! A [`KeyPress`] from its parts, for an interface whose events are not GDK's, and what an
//! interface reads back of a press beyond matching it.
//!
//! The GTK application builds its presses with [`super::normalize`], from a keyval, a
//! keycode, a layout and a level. An interface that has none of those builds the same press
//! from what the normalization would have produced.

use super::{Key, KeyPress, MODIFIER_KEYS, Modifiers, SHORTCUT_MODIFIERS, alias, main_key};

/// What the press typed, for text and for telling a modifier key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    /// One character, as the layout typed it with the held modifiers.
    Character(char),
    /// A key given by its keysym: a named key such as Escape or an arrow, a modifier, or a
    /// keypad key, which GDK reports as itself and not as the character it types.
    Key(Key),
    /// Nothing one key or one character stands for, such as composed text of several
    /// characters.
    Nothing,
}

/// What [`super::normalize`] derives from a GDK event, given directly.
#[derive(Clone, Copy, Debug)]
pub struct KeyPressParts {
    /// The key shortcuts compare. A letter in either case, a keypad key or `ISO_Left_Tab`
    /// may be given: [`KeyPress::from_parts`] lowers, folds and replaces them. Where the
    /// layout types no ASCII on the key, this is already the Latin key it falls back to,
    /// and `fallback` says so.
    pub key: Key,
    /// Ctrl, Alt, Shift and Super as a binding sees them: Shift is left out where it only
    /// selected the symbol the key typed, and kept with a letter. Other flags are dropped.
    pub modifiers: Modifiers,
    /// The same four as they were held before the event. For a modifier's own press that is
    /// without the modifier itself, and for its release it is with it.
    pub held: Modifiers,
    pub typed: Typed,
    /// The digit-row position (1 to 9, then 0) or the keypad digit, whatever the layout
    /// types there; `None` for any other key.
    pub digit: Option<u8>,
    /// Whether `key` came from another layout or from the US table.
    pub fallback: bool,
}

impl KeyPress {
    /// A press as [`super::normalize`] would have made it. A release is built the same way.
    /// No alias is applied: [`super::rewrite`] does that afterwards.
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

    /// The canonical key and modifiers an alias reads as, when the press is one that is no
    /// plain printable key: what an interface hands its toolkit when nothing claimed the
    /// press, so the toolkit acts as it does for the canonical key.
    pub fn unclaimed_alias(&self) -> Option<(Key, Modifiers)> {
        self.alias
            .is_some_and(|alias| !alias.typed)
            .then_some((self.key, self.shortcut_modifiers))
    }
}
