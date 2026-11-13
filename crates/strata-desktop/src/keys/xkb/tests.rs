// SPDX-License-Identifier: MIT

use super::{Xkb, tables};
use crate::{
    key_constants,
    keys::{Backend, Key, Table},
};

macro_rules! declared {
    ($($name:ident = $keysym:literal,)*) => {
        &[$($keysym),*]
    };
}

const DECLARED: &[u32] = key_constants!(declared);

fn key(keysym: u32) -> Key {
    Key::from_keysym(keysym)
}

fn name(keysym: u32) -> Option<String> {
    Xkb.key_name(key(keysym))
}

fn read(name: &str) -> Option<u32> {
    Xkb.key_from_name(name).map(Key::keysym)
}

#[test]
fn a_declared_key_is_named_by_the_table_whatever_the_library_says() {
    assert!(DECLARED.len() > 200, "{} constants", DECLARED.len());
    for keysym in DECLARED {
        let key = key(*keysym);
        let Some(written) = Table.key_name(key) else {
            assert_eq!(key, Key::VoidSymbol);
            continue;
        };
        assert_eq!(Xkb.key_name(key), Some(written.clone()), "{keysym:#x}");
        assert_eq!(Xkb.key_from_name(&written), Some(key), "{written}");
    }
    assert_eq!(name(Key::KP_Page_Down.keysym()).as_deref(), Some("KP_Next"));
    assert_eq!(read("KP_Page_Down"), Some(Key::KP_Page_Down.keysym()));
}

#[test]
fn a_key_without_a_name_is_written_by_its_range_and_never_read_back() {
    let cases = [
        (0x1a4, "0x1a4"),
        (0x0100_0100, "U+0100"),
        (0x0100_0041, "U+0041"),
        (0x0110_ffff, "U+10FFFF"),
        (0x1234_5678, "0x12345678"),
        (Key::VoidSymbol.keysym(), "0xffffff"),
    ];
    for (keysym, written) in cases {
        assert_eq!(name(keysym).as_deref(), Some(written));
        assert_eq!(read(written), None, "{written}");
    }
    assert_eq!(name(0), None);
}

#[test]
fn a_name_gdk_does_not_read_is_refused_here() {
    let refused = [
        "a\0b",
        "U41",
        "U20AC",
        "UD800",
        "0x61",
        "0x0061",
        "XF86U20AC",
        "XF86XF86AudioPlay",
        "XF86_AudioPlay",
        "_AudioPlay",
        "XF86_Switch_VT_1",
        "VoidSymbol",
        "guillemetleft",
        "dead_lowline",
        "osfCopy",
        "XF86Assistant",
        "Assistant",
        "NoSuchKey",
        "",
    ];
    for name in refused {
        assert_eq!(read(name), None, "{name:?}");
    }
}

#[test]
fn a_vendor_key_reads_with_its_prefix_or_without_and_is_written_without() {
    let play = read("AudioPlay").expect("a vendor key");
    assert_eq!(read("XF86AudioPlay"), Some(play));
    assert_eq!(name(play).as_deref(), Some("AudioPlay"));
    assert_eq!(read("SelectButton"), read("XF86SelectButton"));
    assert_eq!(
        read("SelectButton").and_then(name).as_deref(),
        Some("SelectButton")
    );
    assert_ne!(read("Select"), read("SelectButton"));
}

#[test]
fn a_named_key_types_its_character_and_a_character_has_gdks_key() {
    assert_eq!(Xkb.key_to_unicode(Key::Cyrillic_es), Some('с'));
    assert_eq!(Xkb.key_from_unicode('с'), Key::Cyrillic_es);
    assert_eq!(Xkb.key_to_unicode(Key::KP_5), Some('5'));
    assert_eq!(Xkb.key_from_unicode('5'), Key::_5);
    assert_eq!(Xkb.key_to_unicode(Key::Return), Some('\r'));
    assert_eq!(Xkb.key_from_unicode('\r').keysym(), 0x0100_000d);
    assert_eq!(Xkb.key_to_unicode(key(0x20ac)), Some('€'));
    assert_eq!(Xkb.key_from_unicode('€').keysym(), 0x20ac);
    assert_eq!(Xkb.key_to_unicode(key(0x0100_0000)), None);
    assert_eq!(Xkb.key_to_unicode(Key::VoidSymbol), None);
}

#[test]
fn case_goes_through_the_character_and_a_few_keys_pair_by_position() {
    let pairs = [
        (Key::c, Key::C),
        (Key::Cyrillic_es, Key::Cyrillic_ES),
        (Key::agrave, key(0xc0)),
        // A key with no character, which GDK pairs by position.
        (key(0x1b4), key(0x1a4)),
    ];
    for (lower, upper) in pairs {
        assert_eq!(Xkb.key_to_upper(lower), upper, "{lower:?}");
        assert_eq!(Xkb.key_to_lower(upper), lower, "{upper:?}");
        assert_eq!(Xkb.key_to_lower(lower), lower, "{lower:?}");
    }
    // A direct-Unicode keysym lands on its character's own key, in either case.
    assert_eq!(Xkb.key_to_lower(key(0x0100_0041)), Key::a);
    assert_eq!(Xkb.key_to_upper(key(0x0100_0041)), Key::A);
    // Unicode maps these to more than one character, or to none GDK takes.
    assert_eq!(Xkb.key_to_lower(key(0x0100_0130)), Key::i);
    assert_eq!(Xkb.key_to_lower(key(0x0100_2160)).keysym(), 0x0100_2160);
    for unchanged in [Key::_1, Key::Return, Key::F5, Key::VoidSymbol] {
        assert_eq!(Xkb.key_to_lower(unchanged), unchanged);
        assert_eq!(Xkb.key_to_upper(unchanged), unchanged);
    }
}

#[test]
fn the_library_in_use_is_the_one_the_loader_mapped() {
    let library = super::library_in_use().expect("this test links libxkbcommon");
    let name = library.file_name().and_then(|name| name.to_str());
    assert!(
        name.is_some_and(|name| name.starts_with("libxkbcommon.so")),
        "{library:?}"
    );
    assert!(library.is_absolute() && library.exists(), "{library:?}");
}

fn sorted<K: Ord + Copy + std::fmt::Debug, V>(table: &[(K, V)]) -> Result<(), String> {
    match table.windows(2).find(|pair| pair[0].0 >= pair[1].0) {
        Some(pair) => Err(format!("{:?} is before {:?}", pair[0].0, pair[1].0)),
        None => Ok(()),
    }
}

#[test]
fn every_table_is_sorted_as_its_lookup_needs() {
    assert_eq!(sorted(tables::WRITTEN), Ok(()));
    assert_eq!(sorted(tables::READ), Ok(()));
    assert_eq!(sorted(tables::TYPES), Ok(()));
    assert_eq!(sorted(tables::TYPED_BY), Ok(()));
    assert_eq!(sorted(tables::LOWER), Ok(()));
    assert_eq!(sorted(tables::UPPER), Ok(()));
    assert!(tables::WRITTEN_AS_A_NUMBER.is_sorted_by(|a, b| a < b));
    assert!(tables::REFUSED.is_sorted_by(|a, b| a < b));
}
