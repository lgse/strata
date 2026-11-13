// SPDX-License-Identifier: MIT

//! What libxkbcommon says of a keysym, corrected to what GDK says.
//!
//! The rules are here and the single exceptions in [`tables`](super::tables). Both are facts
//! about the two libraries' answers, found by running them side by side.

use xkbcommon::xkb;

use super::tables;

/// The first of the keysyms that stand for a Unicode character directly.
const DIRECT: u32 = 0x0100_0000;
const NO_KEY: u32 = 0;
/// What both libraries answer for a name they do not know.
const VOID: u32 = 0x00ff_ffff;
/// Vendor keys GDK has no name for, where libxkbcommon has: every vendor's but XFree86's,
/// and XFree86's keys named after the Linux kernel's key codes.
const NAMELESS: [std::ops::Range<u32>; 2] = [0x1000_0000..0x1008_0000, 0x1008_1000..0x1008_2000];

fn is_nameless(keysym: u32) -> bool {
    NAMELESS.iter().any(|range| range.contains(&keysym))
}

fn is_direct(keysym: u32) -> bool {
    keysym & 0xff00_0000 == DIRECT
}

/// The entry for `key` in a table sorted by key.
fn found<K: Ord + Copy, V: Copy>(table: &[(K, V)], key: K) -> Option<V> {
    table
        .binary_search_by_key(&key, |(key, _)| *key)
        .ok()
        .map(|index| table[index].1)
}

/// A key GDK has no name for is written as its number, without padding.
fn number(keysym: u32) -> String {
    format!("{keysym:#x}")
}

pub(super) fn name(keysym: u32) -> Option<String> {
    if keysym == NO_KEY {
        return None;
    }
    if is_direct(keysym) {
        return Some(format!("U+{:04X}", keysym & 0x00ff_ffff));
    }
    if let Some(name) = found(tables::WRITTEN, keysym) {
        return Some(name.to_owned());
    }
    if is_nameless(keysym) || tables::WRITTEN_AS_A_NUMBER.binary_search(&keysym).is_ok() {
        return Some(number(keysym));
    }
    let name = xkb::keysym_get_name(xkb::Keysym::new(keysym));
    if name.is_empty() || name.starts_with("0x") {
        return Some(number(keysym));
    }
    Some(match name.strip_prefix("XF86") {
        Some(plain) => plain.to_owned(),
        None => name,
    })
}

/// A keysym or a code point spelled as a number, which libxkbcommon reads as that key and
/// GDK does not read: `0x61`, `U20AC`.
fn is_number(name: &str) -> bool {
    let digits = name
        .strip_prefix("0x")
        .or_else(|| name.strip_prefix("0X"))
        .or_else(|| name.strip_prefix('U'));
    digits.is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|digit| digit.is_ascii_hexdigit())
    })
}

pub(super) fn from_name(name: &str) -> Option<u32> {
    // The binding makes a C string of the name and panics on a NUL.
    if name.contains('\0') {
        return None;
    }
    // GDK reads a vendor key with its prefix or without.
    let plain = name.strip_prefix("XF86").unwrap_or(name);
    if is_number(plain) || tables::REFUSED.binary_search(&plain).is_ok() {
        return None;
    }
    if let Some(keysym) = found(tables::READ, plain) {
        return Some(keysym);
    }
    let known = |name: &str| {
        let keysym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS).raw();
        (keysym != NO_KEY && keysym != VOID).then_some(keysym)
    };
    known(plain)
        .or_else(|| known(&format!("XF86{plain}")))
        .filter(|keysym| !is_nameless(*keysym))
}

pub(super) fn to_unicode(keysym: u32) -> Option<char> {
    if let Some(character) = found(tables::TYPES, keysym) {
        return character;
    }
    let character = xkb::keysym_to_utf32(xkb::Keysym::new(keysym));
    (character != 0)
        .then(|| char::from_u32(character))
        .flatten()
}

pub(super) fn from_unicode(character: char) -> u32 {
    found(tables::TYPED_BY, character)
        .unwrap_or_else(|| xkb::utf32_to_keysym(u32::from(character)).raw())
}

/// Which case a key is taken to.
#[derive(Clone, Copy)]
pub(super) enum Case {
    Lower,
    Upper,
}

/// The one character `character` becomes in `case`, or itself where Unicode gives it none or
/// more than one.
fn in_case(character: char, case: Case) -> char {
    let mapped = match case {
        Case::Lower => single(character.to_lowercase()),
        Case::Upper => single(character.to_uppercase()),
    };
    mapped.unwrap_or(character)
}

fn single(mut characters: impl Iterator<Item = char>) -> Option<char> {
    let first = characters.next()?;
    characters.next().is_none().then_some(first)
}

/// `keysym` in the other case: through the character it types, and a direct-Unicode keysym
/// always onto its character's own keysym. The tables hold the keys GDK pairs by position,
/// with no character behind them, and the characters Unicode maps to more than one.
pub(super) fn in_other_case(keysym: u32, case: Case) -> u32 {
    let pairs = match case {
        Case::Lower => tables::LOWER,
        Case::Upper => tables::UPPER,
    };
    if let Some(other) = found(pairs, keysym) {
        return other;
    }
    let Some(character) = to_unicode(keysym) else {
        return keysym;
    };
    let other = in_case(character, case);
    if other == character && !is_direct(keysym) {
        keysym
    } else {
        from_unicode(other)
    }
}
