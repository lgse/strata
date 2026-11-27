// SPDX-License-Identifier: MIT

use super::{Backend, Key, Modifiers, Table};

mod accelerator;
mod rules;
mod tables;

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

#[cfg(any(test, feature = "test-support"))]
pub fn library_name(keysym: u32) -> Option<String> {
    let name = xkbcommon::xkb::keysym_get_name(xkbcommon::xkb::Keysym::new(keysym));
    (!name.is_empty()).then_some(name)
}

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
