// SPDX-License-Identifier: MIT

//! The key library of the Rune interface: [`Table`] for every key Strata declares, and
//! libxkbcommon for the rest, corrected to what GDK answers.
//!
//! The GTK application asks GDK, and a saved keymap, a label and a press must read the same
//! on both interfaces. libxkbcommon and GDK agree on most keys and differ on some: the
//! difference is in [`rules`], as a few rules and tables of single exceptions. Every entry
//! there is a fact about what the two libraries answer, taken from the output of the GTK
//! application's test `key_lookups`, which compares this backend with GDK over every keysym
//! and fails on a difference the rules do not cover. The rules are exact for the libraries
//! of the test image; `docs/testing.md` says what follows from that.
//!
//! A declared key never reaches the library: the table answers first, so every key the
//! catalog binds reads and writes the same whatever libxkbcommon a user has installed.

use super::{Backend, Key, Modifiers, Table};

mod rules;
mod tables;

/// What libxkbcommon itself writes for a keysym, before any rule: for the test that holds
/// this backend to GDK, which reads back every name either library writes.
#[cfg(any(test, feature = "test-support"))]
pub fn library_name(keysym: u32) -> Option<String> {
    let name = xkbcommon::xkb::keysym_get_name(xkbcommon::xkb::Keysym::new(keysym));
    (!name.is_empty()).then_some(name)
}

/// The backend over [`Table`] and libxkbcommon.
///
/// Like the table, it reads and writes no accelerator string yet:
/// [`Backend::accelerator_parse`] answers `None` and [`Backend::accelerator_name`] panics.
pub struct Xkb;

impl Backend for Xkb {
    fn key_name(&self, key: Key) -> Option<String> {
        Table.key_name(key).or_else(|| rules::name(key.keysym()))
    }

    fn key_from_name(&self, name: &str) -> Option<Key> {
        Table
            .key_from_name(name)
            .or_else(|| rules::from_name(name).map(Key::from_keysym))
    }

    fn key_to_unicode(&self, key: Key) -> Option<char> {
        Table
            .key_to_unicode(key)
            .or_else(|| rules::to_unicode(key.keysym()))
    }

    fn key_from_unicode(&self, character: char) -> Key {
        Key::from_keysym(rules::from_unicode(character))
    }

    fn key_to_lower(&self, key: Key) -> Key {
        Key::from_keysym(rules::in_other_case(key.keysym(), rules::Case::Lower))
    }

    fn key_to_upper(&self, key: Key) -> Key {
        Key::from_keysym(rules::in_other_case(key.keysym(), rules::Case::Upper))
    }

    fn accelerator_parse(&self, accelerator: &str) -> Option<(Key, Modifiers)> {
        accelerator::parse(accelerator)
    }

    fn accelerator_name(&self, key: Key, modifiers: Modifiers) -> String {
        accelerator::name(key, modifiers)
    }
}

/// Where the reader and the writer of accelerator strings go. Neither is written yet.
mod accelerator {
    use super::{Key, Modifiers};

    pub(super) fn parse(_accelerator: &str) -> Option<(Key, Modifiers)> {
        None
    }

    pub(super) fn name(key: Key, modifiers: Modifiers) -> String {
        panic!(
            "the xkbcommon key backend writes no accelerator strings yet, asked for {key:?} \
             with {modifiers:?}"
        )
    }
}

#[cfg(test)]
mod tests;
