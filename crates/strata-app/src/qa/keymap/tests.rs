// SPDX-License-Identifier: MIT

//! The layouts of `tests/fixtures/harness.json`, which the runner's `layout` step names.

use strata_qa::Keymap;

use super::{Layout, Typed, is_plain_us};
use crate::qa::keys::{self, Named};

fn keymap(layouts: &str, variants: &str, group: u8, caps_lock: bool) -> Keymap {
    Keymap {
        layouts: layouts.to_owned(),
        variants: variants.to_owned(),
        group,
        caps_lock,
    }
}

fn layout(layouts: &str, variants: &str, group: u8, caps_lock: bool) -> Layout {
    Layout::compile(&keymap(layouts, variants, group, caps_lock)).expect("the layout compiles")
}

fn character(typed: Option<Typed>) -> Option<char> {
    match typed?.key {
        Named::Character(character) => Some(character),
        _ => None,
    }
}

#[test]
fn only_the_launch_layout_needs_no_compiling() {
    assert!(is_plain_us(&keymap("us", "", 0, false)));
    assert!(!is_plain_us(&keymap("us", "", 0, true)));
    assert!(!is_plain_us(&keymap("us,ru", "", 1, false)));
    assert!(!is_plain_us(&keymap("us", "intl", 0, false)));
}

#[test]
fn a_us_position_types_the_layouts_character() {
    // `ru` in the manifest: the Russian layout as the second group.
    let russian = layout("us,ru", "", 1, false);
    let typed = russian.typed("j", false, false).expect("the J key");
    assert_eq!(typed.key, Named::Character('о'));
    assert_eq!(typed.us, Some('j'), "a command falls back on the US key");
    assert!(!typed.shift);
    assert_eq!(character(russian.typed("J", false, false)), Some('О'));
    assert!(
        russian
            .typed("J", false, false)
            .is_some_and(|typed| typed.shift)
    );
    // The first group of the same keymap is still the US one.
    let first = layout("us,ru", "", 0, false);
    assert_eq!(character(first.typed("j", false, false)), Some('j'));
    // `fr`: AZERTY has A where a US keyboard has Q.
    let french = layout("fr", "", 0, false);
    assert_eq!(character(french.typed("q", false, false)), Some('a'));
}

#[test]
fn keys_that_type_nothing_keep_their_meaning_under_every_layout() {
    let russian = layout("ru", "", 0, false);
    for (name, expected) in [
        ("Return", Named::Enter),
        ("Down", Named::Down),
        ("Escape", Named::Escape),
        ("F2", Named::Function(2)),
        ("BackSpace", Named::Backspace),
        ("Page_Down", Named::PageDown),
        ("KP_Left", Named::Left),
        ("space", Named::Character(' ')),
    ] {
        let typed = russian.typed(name, false, false);
        assert_eq!(typed.map(|typed| typed.key), Some(expected), "{name}");
    }
}

#[test]
fn caps_lock_and_the_level_shift_change_what_a_key_types() {
    // `caps-lock` in the manifest.
    let locked = layout("us", "", 0, true);
    assert_eq!(character(locked.typed("a", false, false)), Some('A'));
    assert_eq!(character(locked.typed("1", false, false)), Some('1'));
    assert!(!locked.has_altgr(), "a US keyboard's right Alt is Alt");
    // `de`: AltGr+Q types the at sign.
    let german = layout("de", "", 0, false);
    assert!(german.has_altgr());
    assert_eq!(character(german.typed("q", false, true)), Some('@'));
    assert_eq!(character(german.typed("q", false, false)), Some('q'));
    assert_eq!(character(german.typed("y", false, false)), Some('z'));
}

#[test]
fn the_compiled_us_layout_types_what_the_built_in_table_does() {
    let us = layout("us", "", 0, false);
    let names = keys::NAMES.iter().map(|(name, _)| (*name).to_owned());
    let characters = "abcxyz0123456789ABCXYZ-=[];',./\\`"
        .chars()
        .map(String::from);
    for name in names.chain(characters) {
        let built_in = keys::named(&name).expect("a key");
        let shift = matches!(built_in, Named::Character(typed) if keys::needs_shift(typed));
        let compiled = us.typed(&name, false, false);
        assert_eq!(
            compiled.map(|typed| (typed.key, typed.shift)),
            Some((built_in, shift)),
            "{name}"
        );
    }
}

#[test]
fn what_no_us_key_types_and_what_does_not_compile_are_told_apart() {
    let us = layout("us", "", 0, false);
    assert_eq!(us.typed("é", false, false), None);
    assert_eq!(us.typed("NoSuchKey", false, false), None);
    assert!(Layout::compile(&keymap("no-such-layout", "", 0, false)).is_err());
    let error = Layout::compile(&keymap("us", "", 1, false));
    assert!(error.is_err(), "a group the keymap lacks is refused");
}
