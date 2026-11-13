// SPDX-License-Identifier: MIT

use super::{DECLARED, Table, UNICODE, WRITTEN_AS, name_of};
use crate::keys::{
    Backend, Binding, Command, Context, Key, KeyPress, KeyPressParts, Keys, Map, Modifiers, Typed,
    bindings::{COPY, FOCUS_PREVIOUS, HIDDEN_FILES},
    install, label_in_map, rewrite,
};

const NONE: Modifiers = Modifiers::empty();
const CTRL: Modifiers = Modifiers::CONTROL_MASK;
const SHIFT: Modifiers = Modifiers::SHIFT_MASK;

#[test]
fn every_declared_key_reads_back_by_its_own_name() {
    assert!(DECLARED.len() > 200, "{} constants", DECLARED.len());
    for (identifier, keysym) in DECLARED {
        let key = Key::from_keysym(*keysym);
        if key == Key::VoidSymbol {
            assert_eq!(Table.key_name(key), None, "no key has no name");
            assert_eq!(Table.key_from_name(name_of(identifier)), None);
            continue;
        }
        let written = Table
            .key_name(key)
            .unwrap_or_else(|| panic!("{identifier} has no name"));
        assert_eq!(Table.key_from_name(&written), Some(key), "{identifier}");
        assert_eq!(
            Table.key_from_name(name_of(identifier)),
            Some(key),
            "{identifier}"
        );
        let renamed = WRITTEN_AS.iter().any(|(other, _)| *other == key);
        assert_eq!(written != name_of(identifier), renamed, "{identifier}");
    }
    assert_eq!(
        Table.key_name(Key::KP_Page_Down).as_deref(),
        Some("KP_Next")
    );
    assert_eq!(Table.key_from_name("1"), Some(Key::_1));
    assert_eq!(Table.key_name(Key::KP_1).as_deref(), Some("KP_1"));
    assert_eq!(Table.key_from_name("_1"), None);
    assert_eq!(Table.key_from_name("NoSuchKey"), None);
    assert_eq!(Table.key_name(Key::from_keysym(0x1008_ff14)), None);
}

#[test]
fn a_character_is_its_latin_1_or_unicode_keysym_and_only_keypad_keys_type_by_name() {
    for code in (0x20..=0x7e).chain(0xa0..=0xff) {
        let character = char::from_u32(code).expect("a Latin-1 character");
        let key = Table.key_from_unicode(character);
        assert_eq!(key.keysym(), code);
        assert_eq!(Table.key_to_unicode(key), Some(character));
    }
    for character in ['я', 'ψ', '€', '\u{1f600}'] {
        let key = Table.key_from_unicode(character);
        assert_eq!(key.keysym(), UNICODE + u32::from(character));
        assert_eq!(Table.key_to_unicode(key), Some(character));
    }
    for (keypad, character) in [
        (Key::KP_5, '5'),
        (Key::KP_Add, '+'),
        (Key::KP_Space, ' '),
        (Key::KP_Decimal, '.'),
        (Key::KP_Separator, ','),
    ] {
        assert_eq!(Table.key_to_unicode(keypad), Some(character), "{keypad:?}");
        assert_ne!(Table.key_from_unicode(character), keypad, "{character:?}");
    }
    for named in [
        Key::Return,
        Key::Escape,
        Key::KP_Enter,
        Key::Cyrillic_es,
        Key::VoidSymbol,
        Key::from_keysym(UNICODE),
    ] {
        assert_eq!(Table.key_to_unicode(named), None, "{named:?}");
    }
}

#[test]
fn only_ascii_letters_have_another_case() {
    assert_eq!(Table.key_to_lower(Key::C), Key::c);
    assert_eq!(Table.key_to_upper(Key::c), Key::C);
    for key in [
        Key::c,
        Key::_1,
        Key::exclam,
        Key::Return,
        Key::agrave,
        Key::Cyrillic_ES,
    ] {
        assert_eq!(Table.key_to_lower(key), key, "{key:?}");
    }
    for key in [Key::C, Key::_1, Key::Return, Key::agrave, Key::Cyrillic_es] {
        assert_eq!(Table.key_to_upper(key), key, "{key:?}");
    }
}

#[test]
fn no_accelerator_string_is_read() {
    for accelerator in ["<Control>c", "F2", ""] {
        assert_eq!(
            Table.accelerator_parse(accelerator),
            None,
            "{accelerator:?}"
        );
    }
}

/// A backend that answers differently from the table, to tell which of two is installed.
struct Other;

impl Backend for Other {
    fn key_name(&self, _: Key) -> Option<String> {
        Some("other".to_owned())
    }

    fn key_from_name(&self, _: &str) -> Option<Key> {
        None
    }

    fn key_to_unicode(&self, _: Key) -> Option<char> {
        None
    }

    fn key_from_unicode(&self, _: char) -> Key {
        Key::VoidSymbol
    }

    fn key_to_lower(&self, key: Key) -> Key {
        key
    }

    fn key_to_upper(&self, key: Key) -> Key {
        key
    }

    fn accelerator_parse(&self, _: &str) -> Option<(Key, Modifiers)> {
        None
    }

    fn accelerator_name(&self, _: Key, _: Modifiers) -> String {
        String::new()
    }
}

fn every_binding_fires(_: Map, _: Command, _: Binding, _: bool) -> bool {
    false
}

fn pressed(key: Key, modifiers: Modifiers, typed: Typed) -> KeyPress {
    let press = KeyPress::from_parts(KeyPressParts {
        key,
        modifiers,
        held: modifiers,
        typed,
        digit: None,
        fallback: false,
    });
    rewrite(press, Context { typing: false })
}

#[test]
fn the_first_install_decides_and_the_catalog_then_works_without_a_library() {
    crate::test_support::in_own_process(
        "keys::table::tests::the_first_install_decides_and_the_catalog_then_works_without_a_library",
        || {
            assert!(install(&Table, every_binding_fires));
            assert!(
                !install(&Other, every_binding_fires),
                "a second install is ignored"
            );
            assert_eq!(Key::Return.name().as_deref(), Some("Return"));

            let copy = pressed(Key::C, CTRL, Typed::Character('c'));
            assert!(copy.is(COPY));
            assert!(!pressed(Key::c, NONE, Typed::Character('c')).is(COPY));
            let back_tab = pressed(Key::ISO_Left_Tab, SHIFT, Typed::Key(Key::ISO_Left_Tab));
            assert!(back_tab.is(FOCUS_PREVIOUS));
            assert_eq!(
                pressed(Key::c, NONE, Typed::Character('c')).text(),
                Some('c')
            );
            let keypad = pressed(Key::KP_5, NONE, Typed::Key(Key::KP_5));
            assert_eq!(keypad.text(), Some('5'), "a keypad digit types its digit");
            // The keypad's decimal key types a period and is not the period key: GDK does not
            // fold it onto the main block, so Ctrl with it is not Ctrl+period.
            let decimal = pressed(Key::KP_Decimal, CTRL, Typed::Key(Key::KP_Decimal));
            assert!(!decimal.is(HIDDEN_FILES.in_map(Map::Default)));
            let period = pressed(Key::period, CTRL, Typed::Character('.'));
            assert!(period.is(HIDDEN_FILES.in_map(Map::Default)));
            let decimal = pressed(Key::KP_Decimal, NONE, Typed::Key(Key::KP_Decimal));
            assert_eq!(decimal.text(), Some('.'));

            assert_eq!(
                label_in_map("en", &Keys::One(COPY.shown()), Map::Default),
                "Ctrl+C"
            );
            assert_eq!(
                label_in_map(
                    "de",
                    &Keys::One(Binding::ctrl_shift(Key::Page_Up).shown()),
                    Map::Default
                ),
                "Strg+Umschalt+Bild auf"
            );
        },
    );
}
