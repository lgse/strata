// SPDX-License-Identifier: MIT

//! The press: a Rune key event as the shared catalog's [`KeyPress`], read as the GTK
//! application reads a GDK event (`normalize` in `strata_desktop::keys`, from
//! `src/ui/keys/mod.rs`). Nothing else in this crate builds one, so a rule about a layout is
//! written here once.
//!
//! Rune gives a logical key and the held modifiers, and, for a key with a physical origin,
//! where the key is ([`Tx::physical_key`]) and what it types with nothing held
//! ([`Tx::key_base`]). From those:
//!
//! - **A named key** is itself.
//! - **A character** is read by `KeyPress::from_signal`, the shared crate's reading of a press
//!   that has no GDK event: the label first, then the US key at the position, the digit row
//!   by position, and the Shift a symbol consumes. The character is always given as
//!   `Key::from_char` gives it, which is what makes that reading right on the table key
//!   backend: the backend has no character for a legacy keysym, and this never passes one.
//! - **What Rune's modifiers do not say** is worked out: Caps Lock from an uppercase
//!   character with no Shift, and a level above Shift's (AltGr) from a character that is
//!   neither the key's base nor typed with Shift.
//!
//! Where this cannot read a press as GDK does is listed in `docs/rune/parity/owner.md`, under
//! B3: another Latin layout that is configured but not active, for which the US position
//! stands in (Rune packet entry 99, `docs/rune/upstream/rune/99-configured-layouts.md`); a
//! level above Shift's while Shift is held; a chord whose level types no ASCII on a key
//! whose base is ASCII, where Rune has dropped the character typed and the US key stands
//! (packet entry 100, `100-chord-typed-character.md`); and a press with no physical origin,
//! which the QA target's all are (packet entry 97).

use rune::{Key as Logical, KeyEvent, Modifier, Mods, PhysicalKeyCode, Tx};
use strata_desktop::keys::{Key, KeyPress, KeyPressParts, Modifiers, Typed};

/// Where a key is and what it types with nothing held, as far as Rune says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Origin {
    pub(super) physical: Option<PhysicalKeyCode>,
    pub(super) base: Option<char>,
}

impl Origin {
    /// The origin of the key a handler is seeing.
    pub(super) fn of(tx: &Tx) -> Self {
        Self {
            physical: tx.physical_key(),
            base: tx.key_base(),
        }
    }
}

/// A keycode no key of the US table has: a press with it gets no Latin fallback and no
/// digit by position, and is still read by what it types.
const OFF_THE_TABLE: u32 = 8;

/// The X keycode of a key of the main block: its evdev code plus 8, which is what GDK
/// reports and what the shared crate's US table is keyed by. A table of positions, not of
/// what any layout types there.
fn x_keycode(physical: PhysicalKeyCode) -> Option<u32> {
    use PhysicalKeyCode::*;
    const ROWS: [(u32, &[PhysicalKeyCode]); 5] = [
        (
            10,
            &[
                Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9, Digit0,
                Minus, Equal,
            ],
        ),
        (
            24,
            &[
                KeyQ,
                KeyW,
                KeyE,
                KeyR,
                KeyT,
                KeyY,
                KeyU,
                KeyI,
                KeyO,
                KeyP,
                BracketLeft,
                BracketRight,
            ],
        ),
        (
            38,
            &[
                KeyA, KeyS, KeyD, KeyF, KeyG, KeyH, KeyJ, KeyK, KeyL, Semicolon, Quote, Backquote,
            ],
        ),
        (51, &[Backslash]),
        (
            52,
            &[
                KeyZ, KeyX, KeyC, KeyV, KeyB, KeyN, KeyM, Comma, Period, Slash,
            ],
        ),
    ];
    ROWS.iter().find_map(|(first, keys)| {
        let along = keys.iter().position(|key| *key == physical)?;
        u32::try_from(along).ok().map(|along| first + along)
    })
}

/// The keypad key at a position, with its digit: what GDK reports for a keypad key that
/// types, where Rune gives the character. Which of its two keys the decimal key is follows
/// the character, as the layout decides it for GDK.
fn keypad(physical: PhysicalKeyCode, typed: char) -> Option<(Key, Option<u8>)> {
    use PhysicalKeyCode::*;
    const DIGITS: [(PhysicalKeyCode, Key); 10] = [
        (Numpad0, Key::KP_0),
        (Numpad1, Key::KP_1),
        (Numpad2, Key::KP_2),
        (Numpad3, Key::KP_3),
        (Numpad4, Key::KP_4),
        (Numpad5, Key::KP_5),
        (Numpad6, Key::KP_6),
        (Numpad7, Key::KP_7),
        (Numpad8, Key::KP_8),
        (Numpad9, Key::KP_9),
    ];
    const OPERATORS: [(PhysicalKeyCode, Key); 5] = [
        (NumpadAdd, Key::KP_Add),
        (NumpadSubtract, Key::KP_Subtract),
        (NumpadMultiply, Key::KP_Multiply),
        (NumpadDivide, Key::KP_Divide),
        (NumpadEqual, Key::KP_Equal),
    ];
    if matches!(physical, NumpadDecimal | NumpadComma) {
        let key = match typed {
            ',' => Key::KP_Separator,
            _ => Key::KP_Decimal,
        };
        return Some((key, None));
    }
    let digit = DIGITS.iter().position(|(code, _)| *code == physical);
    let digit = digit.and_then(|digit| Some((DIGITS[digit].1, Some(u8::try_from(digit).ok()?))));
    let operator = OPERATORS.iter().find(|(code, _)| *code == physical);
    digit.or_else(|| operator.map(|(_, key)| (*key, None)))
}

/// Ctrl, Alt, Shift and Super as held. Off macOS Rune's primary modifier is Ctrl again, and
/// its `logo` is Super, Hyper and Meta alike.
fn held(mods: Mods) -> Modifiers {
    let mut held = Modifiers::empty();
    held.set(Modifiers::SHIFT_MASK, mods.shift);
    held.set(Modifiers::CONTROL_MASK, mods.ctrl || mods.cmd);
    held.set(Modifiers::ALT_MASK, mods.alt);
    held.set(Modifiers::SUPER_MASK, mods.logo);
    held
}

const FUNCTION_KEYS: [Key; 24] = [
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
    Key::F13,
    Key::F14,
    Key::F15,
    Key::F16,
    Key::F17,
    Key::F18,
    Key::F19,
    Key::F20,
    Key::F21,
    Key::F22,
    Key::F23,
    Key::F24,
];

/// The key of a logical key that is no character.
fn named(key: &Logical) -> Option<Key> {
    Some(match key {
        Logical::Enter => Key::Return,
        Logical::Backspace => Key::BackSpace,
        Logical::Delete => Key::Delete,
        Logical::Left => Key::Left,
        Logical::Right => Key::Right,
        Logical::Up => Key::Up,
        Logical::Down => Key::Down,
        Logical::Home => Key::Home,
        Logical::End => Key::End,
        Logical::PageUp => Key::Page_Up,
        Logical::PageDown => Key::Page_Down,
        Logical::Insert => Key::Insert,
        Logical::PrintScreen => Key::Print,
        Logical::ScrollLock => Key::Scroll_Lock,
        Logical::Pause => Key::Pause,
        Logical::Escape => Key::Escape,
        Logical::Tab => Key::Tab,
        Logical::ContextMenu => Key::Menu,
        Logical::Function(number) => *FUNCTION_KEYS.get(usize::from(number.checked_sub(1)?))?,
        _ => return None,
    })
}

fn modifier_key(modifier: Modifier) -> Option<Key> {
    Some(match modifier {
        Modifier::ShiftLeft => Key::Shift_L,
        Modifier::ShiftRight => Key::Shift_R,
        Modifier::ControlLeft => Key::Control_L,
        Modifier::ControlRight => Key::Control_R,
        Modifier::AltLeft => Key::Alt_L,
        Modifier::AltRight => Key::Alt_R,
        Modifier::SuperLeft => Key::Super_L,
        Modifier::SuperRight => Key::Super_R,
        _ => return None,
    })
}

/// A press of `key`, which is no character of the main block: it matches as itself.
fn of_key(key: Key, held: Modifiers, digit: Option<u8>) -> KeyPress {
    KeyPress::from_parts(KeyPressParts {
        key,
        modifiers: held,
        held,
        typed: Typed::Key(key),
        digit,
        fallback: false,
    })
}

/// A press that is no key and no single character: composed text, a key Rune adds later.
fn of_nothing(held: Modifiers) -> KeyPress {
    KeyPress::from_parts(KeyPressParts {
        key: Key::VoidSymbol,
        modifiers: held,
        held,
        typed: Typed::Nothing,
        digit: None,
        fallback: false,
    })
}

/// A modifier's own press or release. GDK reports the modifiers held before the event, and
/// Rune those after it: the key's own is taken out of a press and put into a release.
fn of_modifier(modifier: Modifier, pressed: bool, held: Modifiers) -> KeyPress {
    let Some(key) = modifier_key(modifier) else {
        return of_nothing(held);
    };
    let own = of_key(key, Modifiers::empty(), None).modifier_key();
    let before = if pressed { held - own } else { held | own };
    of_key(key, before, None)
}

fn same_letter(typed: char, base: char) -> bool {
    typed == base || typed.to_lowercase().eq(base.to_lowercase())
}

/// What the key typed, where Rune named another. With Ctrl held, Rune's host puts the US key
/// in the place of a character that is not ASCII and drops the character. Where the key's
/// base is not ASCII either, the base is what GDK would have delivered. Where the base is
/// ASCII nothing says a character was replaced, and the US key stands.
fn as_laid_out(named: char, command: bool, base: Option<char>) -> char {
    match base {
        Some(base) if command && named.is_ascii() && !base.is_ascii() => base,
        _ => named,
    }
}

/// Whether `typed` is above the level Shift selects (AltGr): it is neither the key's base
/// nor typed with Shift. Such a character is text, with no fallback and no digit by
/// position. With Shift held the two levels cannot be told apart.
fn above_shifts_level(typed: char, mods: Mods, base: Option<char>) -> bool {
    !mods.shift && base.is_some_and(|base| !same_letter(typed, base))
}

/// The keycode the shared reading takes the position from.
fn keycode(origin: Origin, above: bool) -> u32 {
    match origin.physical {
        // No physical origin: the US key that types the character.
        None => 0,
        Some(_) if above => OFF_THE_TABLE,
        Some(physical) => x_keycode(physical).unwrap_or(OFF_THE_TABLE),
    }
}

/// One character, typed or with Ctrl held (`command`, where Rune has lowered it and may have
/// put the US key in its place).
fn of_character(named: char, command: bool, mods: Mods, origin: Origin) -> KeyPress {
    let mut held = held(mods);
    if let Some((key, digit)) = origin.physical.and_then(|physical| keypad(physical, named)) {
        return of_key(key, held, digit);
    }
    let typed = as_laid_out(named, command, origin.base);
    // Caps Lock, which Rune's modifiers do not carry, types the capital with no Shift.
    if typed.is_uppercase() && !mods.shift {
        held |= Modifiers::LOCK_MASK;
    }
    let above = above_shifts_level(typed, mods, origin.base);
    KeyPress::from_signal(Key::from_char(typed), keycode(origin, above), held)
}

/// The press `event` is, before any alias is read (`strata_desktop::keys::rewrite`).
pub(super) fn read(event: &KeyEvent, origin: Origin) -> KeyPress {
    let mods = event.mods;
    match &event.key {
        Logical::Command(character) => of_character(*character, true, mods, origin),
        Logical::Text(text) => {
            let mut characters = text.chars();
            match (characters.next(), characters.next()) {
                (Some(character), None) => of_character(character, false, mods, origin),
                _ => of_nothing(held(mods)),
            }
        }
        Logical::Modifier { modifier, pressed } => of_modifier(*modifier, *pressed, held(mods)),
        key => named(key).map_or_else(
            || of_nothing(held(mods)),
            |key| of_key(key, held(mods), None),
        ),
    }
}

#[cfg(test)]
mod tests;
