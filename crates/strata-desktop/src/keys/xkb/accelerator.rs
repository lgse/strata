// SPDX-License-Identifier: MIT

use super::{Backend, Key, Modifiers, Xkb};

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
