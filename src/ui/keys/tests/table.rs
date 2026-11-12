// SPDX-License-Identifier: MIT

//! The table backend of `strata_desktop::keys`, which the Rune interface installs, held to
//! GDK for everything it claims to answer.

use gtk::gdk;
use strata_desktop::keys::{Backend, Table};

use super::{
    super::{gdk_key, key_from_gdk},
    constants::CONSTANTS,
    *,
};

#[test]
fn the_table_backend_names_every_declared_key_as_gdk_does() {
    let mut differences = Vec::new();
    for (constant, keysym) in CONSTANTS {
        let key = Key::from_keysym(*keysym);
        if key == Key::VoidSymbol {
            // No key: the table gives it no name, where GDK writes it as a number, the form
            // of a key without a name, which the table does not write for any key.
            assert_eq!(Table.key_name(key), None);
            assert_eq!(Table.key_from_name(constant), None);
            assert_eq!(gdk::Key::from_name(*constant), None);
            continue;
        }
        let written = Table.key_name(key);
        let gdk_written = gdk_key(key).name().map(String::from);
        if written != gdk_written {
            differences.push(format!(
                "{constant}: writes {written:?}, GDK {gdk_written:?}"
            ));
        }
        for name in written.iter().chain(&gdk_written) {
            let read = Table.key_from_name(name);
            let gdk_read = gdk::Key::from_name(name).map(key_from_gdk);
            if read != gdk_read {
                differences.push(format!("{name}: reads {read:?}, GDK {gdk_read:?}"));
            }
        }
    }
    assert!(differences.is_empty(), "{differences:#?}");
}

#[test]
fn the_table_backend_types_what_gdk_does_for_a_declared_key_or_nothing() {
    let mut typed = 0;
    for (constant, keysym) in CONSTANTS {
        let key = Key::from_keysym(*keysym);
        let Some(character) = Table.key_to_unicode(key) else {
            continue;
        };
        typed += 1;
        assert_eq!(Some(character), gdk_key(key).to_unicode(), "{constant}");
    }
    assert!(typed > 100, "{typed} declared keys type a character");
    for keypad in [
        Key::KP_Space,
        Key::KP_Equal,
        Key::KP_Multiply,
        Key::KP_Add,
        Key::KP_Subtract,
        Key::KP_Divide,
        Key::KP_0,
        Key::KP_1,
        Key::KP_2,
        Key::KP_3,
        Key::KP_4,
        Key::KP_5,
        Key::KP_6,
        Key::KP_7,
        Key::KP_8,
        Key::KP_9,
    ] {
        let character = gdk_key(keypad).to_unicode();
        assert!(
            character.is_some_and(|character| !character.is_control()),
            "{keypad:?}"
        );
        assert_eq!(Table.key_to_unicode(keypad), character, "{keypad:?}");
    }
    for code in [0, 0x41, 0xe9, 0x44f, 0x20ac, 0x1_f600, 0x10_ffff] {
        let key = Key::from_keysym(0x0100_0000 + code);
        assert_eq!(
            Table.key_to_unicode(key),
            gdk_key(key).to_unicode(),
            "{code:#x}"
        );
    }
}

#[test]
fn the_table_backend_reads_latin_1_characters_and_ascii_case_as_gdk_does() {
    for code in (0x20..=0x7e).chain(0xa0..=0xff) {
        let character = char::from_u32(code).expect("a Latin-1 character");
        let key = Key::from_keysym(code);
        assert_eq!(
            Table.key_from_unicode(character),
            Key::from_keysym(gdk::unicode_to_keyval(code)),
            "{character:?}"
        );
        assert_eq!(
            Table.key_to_unicode(key),
            gdk_key(key).to_unicode(),
            "{code:#x}"
        );
    }
    for code in 0x20..=0x7e {
        let key = Key::from_keysym(code);
        assert_eq!(
            Table.key_to_lower(key),
            key_from_gdk(gdk_key(key).to_lower()),
            "{code:#x}"
        );
        assert_eq!(
            Table.key_to_upper(key),
            key_from_gdk(gdk_key(key).to_upper()),
            "{code:#x}"
        );
    }
}
