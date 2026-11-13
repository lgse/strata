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
//! A declared key's name, read or written, and the character it types never reach the
//! library: the table answers first, so every key the catalog binds reads and writes the
//! same whatever libxkbcommon a user has installed. The key of a character and a key's
//! other case do go through the rules and the library, for a declared key as for any.
//!
//! Accelerator strings, which a saved keymap holds, are read and written in [`accelerator`]
//! on top of those lookups.

use super::{Backend, Key, Modifiers, Table};

mod accelerator;
mod rules;
mod tables;

/// The libxkbcommon this process runs on, as the file the loader mapped: its name carries
/// the library's version where the distribution's does (`libxkbcommon.so.0.13.2`). The
/// library has no call that tells its version. For a log line: the tables are exact for one
/// version, and a report that a key reads differently on the two interfaces starts here.
pub fn library_in_use() -> Option<std::path::PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    maps.lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .map(std::path::PathBuf::from)
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libxkbcommon.so"))
        })
}

/// What libxkbcommon itself writes for a keysym, before any rule: for the test that holds
/// this backend to GDK, which reads back every name either library writes.
#[cfg(any(test, feature = "test-support"))]
pub fn library_name(keysym: u32) -> Option<String> {
    let name = xkbcommon::xkb::keysym_get_name(xkbcommon::xkb::Keysym::new(keysym));
    (!name.is_empty()).then_some(name)
}

/// The backend over [`Table`] and libxkbcommon.
///
/// It reads and writes accelerator strings in GTK's format, through its own key lookups
/// ([`accelerator`]), so the accelerator strings of a saved keymap read and write as GTK's
/// do. Whether an interface reads a saved keymap with it is the interface's.
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

#[cfg(test)]
mod tests;
