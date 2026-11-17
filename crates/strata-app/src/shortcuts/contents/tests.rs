// SPDX-License-Identifier: MIT

//! The port against the catalog, as GTK's `every_listed_binding_has_its_own_row_in_its_maps`
//! (`src/ui/keys/tests/catalog.rs`) holds GTK's tables to it: each command the catalog lists
//! in a map has a row of its own in some view's sections of that map. Both sides answer to the
//! same catalog, so a command added there fails here until the port has its row.

use strata_desktop::keys::{
    Binding, CUSTOM_ACTIONS, CommandKind as Kind, Digits, Key, Keymap, Keys, Letters, Map,
    Modifiers, Shown,
    bindings::{self, Entry, Listed},
    registry, use_keymap,
};

use super::{Mention, Mode, Shortcut, default_sections};
use crate::shortcuts::tenxer::tenxer_sections;

/// What a row shows for a listed constant: its bindings, the chord key they follow, or its
/// digit or letter range (GTK's `Wanted`).
#[derive(PartialEq)]
enum Wanted {
    Keys(Vec<Binding>, Option<Binding>),
    Digits(Digits),
    Letters(Letters),
}

fn wanted(constant: &Listed, map: Map) -> Wanted {
    match constant.entry {
        Entry::Binding(binding) => Wanted::Keys(vec![binding], None),
        Entry::Digits(digits) => Wanted::Digits(digits),
        Entry::Letters(letters) => Wanted::Letters(letters),
        Entry::Command(command) => {
            let after = match command.kind() {
                Kind::Step(group) => {
                    registry().find(|trigger| trigger.kind() == Kind::Trigger(group))
                }
                Kind::Single | Kind::Trigger(_) | Kind::Structural => constant.after,
            };
            Wanted::Keys(
                command.bindings_in(map),
                after.and_then(|after| after.bindings_in(map).first().copied()),
            )
        }
    }
}

/// GTK's `shows`, in `map`.
fn shows(keys: &Keys, wanted: &Wanted, map: Map) -> bool {
    let primary = |shown: &Shown| shown.bindings(map).first().copied();
    match (keys, wanted) {
        (Keys::Mixed(alternatives), _) => alternatives.iter().any(|keys| shows(keys, wanted, map)),
        (Keys::Digits(digits), Wanted::Digits(wanted)) => digits == wanted,
        (Keys::Letters(letters), Wanted::Letters(wanted)) => letters == wanted,
        (Keys::Letters(letters), Wanted::Keys(bindings, None)) => letters
            .prefix()
            .as_ref()
            .and_then(primary)
            .is_some_and(|prefix| bindings.contains(&prefix)),
        (Keys::Digits(digits), Wanted::Keys(bindings, Some(first))) => {
            digits.prefix().as_ref().and_then(primary) == Some(*first)
                && digits
                    .then()
                    .as_ref()
                    .and_then(primary)
                    .is_some_and(|then| bindings.contains(&then))
        }
        (Keys::Chord(steps), Wanted::Keys(bindings, Some(first))) => {
            let steps: Vec<Option<Binding>> = steps.iter().map(primary).collect();
            bindings.iter().any(|binding| {
                steps
                    .windows(2)
                    .any(|pair| pair == [Some(*first), Some(*binding)])
            })
        }
        (Keys::Chord(steps), Wanted::Keys(bindings, None)) => steps
            .first()
            .and_then(primary)
            .is_some_and(|step| bindings.contains(&step)),
        (Keys::One(shown), Wanted::Keys(bindings, None)) => shown
            .bindings(map)
            .iter()
            .any(|binding| bindings.contains(binding)),
        (Keys::Any(shown), Wanted::Keys(bindings, None)) => shown
            .iter()
            .flat_map(|shown| shown.bindings(map))
            .any(|binding| bindings.contains(&binding)),
        _ => false,
    }
}

/// 10xer's constants only the portal chooser's sections list (`chooser_requests`,
/// `chooser_files`), which are A20's to port.
const CHOOSER_ONLY: &[&str] = &["CHOOSE_FOLDER"];

/// Custom action slots ship no key, so bind each to find its row.
fn keymap_with_custom_actions_bound() -> Keymap {
    let slot_keys = [
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
    ];
    CUSTOM_ACTIONS
        .iter()
        .zip(slot_keys)
        .fold(Keymap::defaults(), |keymap, (slot, key)| {
            let binding = Binding::new(key, Modifiers::ALT_MASK | Modifiers::SUPER_MASK);
            keymap.with(Map::Default, *slot, vec![binding])
        })
}

/// Every view's rows in `map`, each once, with the keys its action names (GTK's
/// `reference_rows`).
fn rows(map: Map) -> Vec<Vec<Keys>> {
    let mut rows: Vec<Shortcut> = Vec::new();
    for mode in [Mode::Columns, Mode::Icons, Mode::List] {
        let sections = match map {
            Map::Tenxer => tenxer_sections(mode),
            Map::Default => default_sections(mode),
        };
        for shortcut in sections.into_iter().flat_map(|s| s.rows) {
            if !rows.contains(&shortcut) {
                rows.push(shortcut);
            }
        }
    }
    rows.into_iter()
        .map(|shortcut| {
            let mentioned = match shortcut.mention {
                Some(Mention::Keys(keys)) => Some(keys),
                Some(Mention::Held(_)) | None => None,
            };
            std::iter::once(shortcut.keys).chain(mentioned).collect()
        })
        .collect()
}

#[test]
fn every_listed_binding_has_its_own_row_in_its_maps() {
    crate::startup::install_key_library().expect("the key library");
    let _keymap = use_keymap(keymap_with_custom_actions_bound());
    for map in [Map::Default, Map::Tenxer] {
        assert_each_listed_constant_has_a_row(map);
    }
}

fn assert_each_listed_constant_has_a_row(map: Map) {
    let tenxer = map == Map::Tenxer;
    let rows = rows(map);
    let wanted: Vec<(Listed, Wanted)> = bindings::listed()
        .filter(|constant| constant.maps.include(tenxer))
        .filter(|constant| !(tenxer && CHOOSER_ONLY.contains(&constant.name)))
        .map(|constant| {
            let keys = wanted(&constant, map);
            (constant, keys)
        })
        .collect();
    assert!(!wanted.is_empty());
    for (constant, keys) in &wanted {
        let sharing: Vec<&str> = wanted
            .iter()
            .filter(|(_, other)| other == keys)
            .map(|(other, _)| other.name)
            .collect();
        let shown = rows
            .iter()
            .filter(|row| row.iter().any(|row_keys| shows(row_keys, keys, map)))
            .count();
        assert!(
            shown >= sharing.len(),
            "{sharing:?} need a row each in the {map:?} reference, which has {shown}: \
             port GTK's row from src/ui/shortcut_reference.rs ({})",
            constant.name
        );
    }
}
