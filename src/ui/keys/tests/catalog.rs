// SPDX-License-Identifier: MIT

use gtk::glib::translate::IntoGlib;

use super::super::{
    CUSTOM_ACTIONS, Keymap, Keys, Letters, Map, Shown,
    bindings::{self, Entry, Listed},
    command::{Kind, registry},
    main_key, use_keymap,
};
use super::*;

#[test]
fn catalog_bindings_use_the_canonical_key() {
    let mut checked = 0;
    for entry in bindings::entries() {
        let (name, bindings) = match entry {
            Entry::Binding(binding) => (format!("{binding:?}"), vec![binding]),
            Entry::Command(command) => (
                command.id().to_owned(),
                [Map::Default, Map::Tenxer]
                    .into_iter()
                    .flat_map(|map| command.defaults(map).iter().copied())
                    .collect(),
            ),
            Entry::Digits(_) | Entry::Letters(_) => continue,
        };
        for binding in bindings {
            checked += 1;
            let key = binding.key();
            assert_eq!(key, key.to_lower(), "{name} names an uppercase letter");
            assert_eq!(key, main_key(key), "{name} names a keypad key");
            assert_ne!(
                key,
                Key::ISO_Left_Tab,
                "{name} spells Shift+Tab as ISO_Left_Tab"
            );
            let symbol = (0x21..=0x7e).contains(&key.into_glib())
                && !key.to_unicode().is_some_and(|c| c.is_ascii_alphanumeric());
            assert!(
                !(symbol && binding.modifiers().contains(SHIFT)),
                "{name} adds the Shift that types its symbol"
            );
        }
    }
    assert!(checked > 0);
}

/// What a row shows for a listed constant in a map: its bindings, the chord
/// key they follow, or its digit or letter range.
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

fn shows(keys: &Keys, wanted: &Wanted, map: Map) -> bool {
    let primary = |shown: &Shown| shown.bindings(map).first().copied();
    match (keys, wanted) {
        (Keys::Mixed(alternatives), _) => alternatives.iter().any(|keys| shows(keys, wanted, map)),
        (Keys::Digits(digits), Wanted::Digits(wanted)) => digits == wanted,
        (Keys::Letters(letters), Wanted::Letters(wanted)) => letters == wanted,
        // A letter range shows the trigger it follows.
        (Keys::Letters(letters), Wanted::Keys(bindings, None)) => letters
            .prefix()
            .as_ref()
            .and_then(primary)
            .is_some_and(|prefix| bindings.contains(&prefix)),
        // The digits after a two-key chord show its second key too: ; . 1–9.
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

fn assert_each_listed_constant_has_a_row(listed: &[Listed], map: Map) {
    let tenxer = map == Map::Tenxer;
    let rows = crate::ui::shortcut_reference::tests::reference_rows(tenxer);
    let wanted: Vec<(&Listed, Wanted)> = listed
        .iter()
        .filter(|constant| constant.maps.include(tenxer))
        .map(|constant| (constant, wanted(constant, map)))
        .collect();
    for (constant, keys) in &wanted {
        assert!(
            !matches!(keys, Wanted::Keys(bindings, _) if bindings.is_empty()),
            "{} is listed in the {map:?} map but has no binding there",
            constant.name
        );
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
            "{sharing:?} need a row each in the {map:?} reference, which has {shown}; add \
             a row, or move a constant to the catalog block of the maps that list it"
        );
    }
}

/// Constants with equal keys each need a row of their own, since a row shows
/// bindings rather than constants. A command needs its row in each map its
/// catalog block lists it in.
#[test]
fn every_listed_binding_has_its_own_row_in_its_maps() {
    let _keymap = use_keymap(keymap_with_custom_actions_bound());
    let listed: Vec<Listed> = bindings::listed().collect();
    assert!(!listed.is_empty());
    for map in [Map::Default, Map::Tenxer] {
        assert_each_listed_constant_has_a_row(&listed, map);
    }
}
