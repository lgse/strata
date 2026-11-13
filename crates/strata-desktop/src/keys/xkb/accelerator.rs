// SPDX-License-Identifier: MIT

//! Accelerator strings, read as `gtk_accelerator_parse` reads them and written as
//! `gtk_accelerator_name` writes them.
//!
//! Written from GTK's reference manual and from GTK's answers to chosen inputs, never from its
//! source. The manual gives the format, `<Control>a` or `<Shift><Alt>F1`: modifiers in angle
//! brackets, in any case and with some abbreviations, then a key name as GDK reads it. What it
//! leaves open was found by asking GTK:
//!
//! - **The modifier words** are the ten in [`READ`], in any ASCII case and nothing else: no
//!   other abbreviation, no `<Release>`, `<Lock>` or `<Mod1>`, no space inside the brackets.
//!   One unknown word makes the whole string unreadable. A word may repeat.
//! - **The key** is everything after the last modifier, read whole by the key lookups. So no
//!   space or other character may stand around it, a character such as `-` is not a key name,
//!   and a number (`0x61`, `U+0041`) is not read, as a key name is not. It is then taken to
//!   lower case, so `A` reads as plain `a` and `Eacute` as `eacute`, without Shift.
//! - **No key at all** reads: the empty string is no key and no modifiers, and `<Control>` is
//!   no key held with Control. The caller decides what that is worth.
//! - **A NUL** ends the string, because GTK is handed a C string.
//!
//! The writer writes the modifiers in [`WRITTEN`]'s order, with those names, and leaves out
//! Caps Lock; then the name of the key's lower case, or nothing for a key with no name. A key
//! the lookups write as a number (`0x1b4`, `U+10FFFF`) is written so and is not read back.
//!
//! The GTK application's test `key_lookups` holds both functions to GTK over a generated
//! domain, and `accelerator/tests.rs` holds what needs no GTK.

use super::{Backend, Key, Modifiers, Xkb};

/// The words between angle brackets that the reader takes, in lower case, with the modifier
/// each stands for. `primary` is Control, as GTK reads it on Linux.
const READ: [(&str, Modifiers); 10] = [
    ("shift", Modifiers::SHIFT_MASK),
    ("shft", Modifiers::SHIFT_MASK),
    ("control", Modifiers::CONTROL_MASK),
    ("ctrl", Modifiers::CONTROL_MASK),
    ("ctl", Modifiers::CONTROL_MASK),
    ("primary", Modifiers::CONTROL_MASK),
    ("alt", Modifiers::ALT_MASK),
    ("meta", Modifiers::META_MASK),
    ("super", Modifiers::SUPER_MASK),
    ("hyper", Modifiers::HYPER_MASK),
];

/// The modifiers the writer writes, in its order. Meta comes before Super.
const WRITTEN: [(Modifiers, &str); 6] = [
    (Modifiers::SHIFT_MASK, "<Shift>"),
    (Modifiers::CONTROL_MASK, "<Control>"),
    (Modifiers::ALT_MASK, "<Alt>"),
    (Modifiers::META_MASK, "<Meta>"),
    (Modifiers::SUPER_MASK, "<Super>"),
    (Modifiers::HYPER_MASK, "<Hyper>"),
];

const NO_KEY: Key = Key::from_keysym(0);

fn modifier(word: &str) -> Option<Modifiers> {
    READ.iter()
        .find(|(read, _)| read.eq_ignore_ascii_case(word))
        .map(|(_, modifier)| *modifier)
}

pub(super) fn parse(accelerator: &str) -> Option<(Key, Modifiers)> {
    let accelerator = accelerator.split('\0').next().unwrap_or_default();
    let mut modifiers = Modifiers::empty();
    let mut rest = accelerator;
    while let Some(bracketed) = rest.strip_prefix('<') {
        let (word, after) = bracketed.split_once('>')?;
        modifiers |= modifier(word)?;
        rest = after;
    }
    if rest.is_empty() {
        return Some((NO_KEY, modifiers));
    }
    let key = Xkb.key_from_name(rest)?;
    Some((Xkb.key_to_lower(key), modifiers))
}

pub(super) fn name(key: Key, modifiers: Modifiers) -> String {
    let mut written: String = WRITTEN
        .iter()
        .filter(|(modifier, _)| modifiers.contains(*modifier))
        .map(|(_, written)| *written)
        .collect();
    if let Some(name) = Xkb.key_name(Xkb.key_to_lower(key)) {
        written.push_str(&name);
    }
    written
}

#[cfg(test)]
mod tests;
