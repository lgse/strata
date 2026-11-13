// SPDX-License-Identifier: MIT

//! Each expected answer here is GTK's own for the same input, as `gtk_accelerator_parse` and
//! `gtk_accelerator_name` gave it; the GTK application's test `key_lookups` checks them all
//! against GTK over a far wider domain.

use super::{NO_KEY, name, parse};
use crate::{
    key_constants,
    keys::{Backend, Key, Modifiers, Xkb},
};

const SHIFT: Modifiers = Modifiers::SHIFT_MASK;
const CONTROL: Modifiers = Modifiers::CONTROL_MASK;
const ALT: Modifiers = Modifiers::ALT_MASK;
const SUPER: Modifiers = Modifiers::SUPER_MASK;
const HYPER: Modifiers = Modifiers::HYPER_MASK;
const META: Modifiers = Modifiers::META_MASK;

fn key(keysym: u32) -> Key {
    Key::from_keysym(keysym)
}

fn assert_reads(cases: &[(&str, Option<(Key, Modifiers)>)]) {
    for (accelerator, read) in cases {
        assert_eq!(parse(accelerator), *read, "{accelerator:?}");
    }
}

#[test]
fn ten_modifier_words_read_in_any_ascii_case_and_nothing_else_does() {
    let a = Key::a;
    assert_reads(&[
        ("<Shift>a", Some((a, SHIFT))),
        ("<sHIFT>a", Some((a, SHIFT))),
        ("<Shft>a", Some((a, SHIFT))),
        ("<Control>a", Some((a, CONTROL))),
        ("<cOnTrOl>a", Some((a, CONTROL))),
        ("<Ctrl>a", Some((a, CONTROL))),
        ("<CTL>a", Some((a, CONTROL))),
        ("<Primary>a", Some((a, CONTROL))),
        ("<ALT>a", Some((a, ALT))),
        ("<meta>a", Some((a, META))),
        ("<Super>a", Some((a, SUPER))),
        ("<HYPER>a", Some((a, HYPER))),
        ("<Control><Control>a", Some((a, CONTROL))),
        (
            "<Hyper><Super><Meta><Alt><Ctl><Shft>a",
            Some((a, SHIFT | CONTROL | ALT | META | SUPER | HYPER)),
        ),
    ]);
    let refused = [
        "<Cntrl>a",
        "<Sft>a",
        "<Controls>a",
        "<Contro>a",
        "<Mod1>a",
        "<Mod4>a",
        "<Lock>a",
        "<Release>a",
        "<Button1>a",
        "<AltGr>a",
        "<>a",
        "< Control>a",
        "<Control >a",
        "<\u{17f}hift>a",
        "<Sh\u{131}ft>a",
        "<SH\u{130}FT>a",
        "<Control><Foo>a",
    ];
    for accelerator in refused {
        assert_eq!(parse(accelerator), None, "{accelerator:?}");
    }
}

#[test]
fn the_rest_is_one_key_name_taken_to_lower_case_without_shift() {
    assert_reads(&[
        ("A", Some((Key::a, Modifiers::empty()))),
        ("<Control>Z", Some((Key::z, CONTROL))),
        ("<Shift>A", Some((Key::a, SHIFT))),
        ("<Control>Eacute", Some((key(0xe9), CONTROL))),
        ("<Control>Cyrillic_ES", Some((Key::Cyrillic_es, CONTROL))),
        ("<Control>Prior", Some((Key::Page_Up, CONTROL))),
        ("<Control>XF86AudioPlay", Some((key(0x1008_ff14), CONTROL))),
        ("<Control>ISO_Left_Tab", Some((Key::ISO_Left_Tab, CONTROL))),
        ("<Control>Control_L", Some((Key::Control_L, CONTROL))),
    ]);
    let refused = [
        "<Control>-",
        "<Control>?",
        "<Control>return",
        "<Control>EACUTE",
        "<Control>é",
        "<Control>0x61",
        "<Control>U+0041",
        "<Control>0x1b4",
        "<Control>VoidSymbol",
        "<Control>ab",
        "<Control> a",
        "<Control>a ",
        " <Control>a",
        "<Control>a<Shift>",
        "a<Control>",
        "<Control>>a",
        "<<Control>a",
        "<Control",
        "Control>a",
    ];
    for accelerator in refused {
        assert_eq!(parse(accelerator), None, "{accelerator:?}");
    }
}

#[test]
fn no_key_reads_as_key_zero_and_a_nul_ends_the_string() {
    assert_reads(&[
        ("", Some((NO_KEY, Modifiers::empty()))),
        ("<Control>", Some((NO_KEY, CONTROL))),
        ("<Control><Shift>", Some((NO_KEY, SHIFT | CONTROL))),
        ("\0", Some((NO_KEY, Modifiers::empty()))),
        ("<Control>\0a", Some((NO_KEY, CONTROL))),
        ("<Control>a\0b", Some((Key::a, CONTROL))),
        ("<Con\0trol>a", None),
    ]);
}

#[test]
fn the_writer_orders_and_spells_modifiers_as_gtk_and_names_the_lower_case() {
    let cases = [
        (Key::a, Modifiers::empty(), "a"),
        (Key::A, SHIFT, "<Shift>a"),
        (Key::a, CONTROL | SHIFT, "<Shift><Control>a"),
        (Key::a, SUPER | META, "<Meta><Super>a"),
        (Key::a, HYPER | META, "<Meta><Hyper>a"),
        (Key::a, Modifiers::all(), "<Shift><Control><Alt><Meta><Super><Hyper>a"),
        (Key::a, Modifiers::LOCK_MASK | ALT, "<Alt>a"),
        (Key::Cyrillic_ES, CONTROL, "<Control>Cyrillic_es"),
        (key(0xc9), CONTROL, "<Control>eacute"),
        (Key::KP_Page_Down, Modifiers::empty(), "KP_Next"),
        (key(0x0100_0100), CONTROL, "<Control>amacron"),
        (key(0x0100_0041), Modifiers::empty(), "a"),
        // No name of its own: the number of its lower case, which is not read back.
        (key(0x1a4), CONTROL, "<Control>0x1b4"),
        (key(0x0110_ffff), Modifiers::empty(), "U+10FFFF"),
        (Key::VoidSymbol, Modifiers::empty(), "0xffffff"),
        (NO_KEY, Modifiers::empty(), ""),
        (NO_KEY, CONTROL | SHIFT, "<Shift><Control>"),
    ];
    for (key, modifiers, written) in cases {
        assert_eq!(name(key, modifiers), written, "{key:?} {modifiers:?}");
    }
}

macro_rules! declared {
    ($($name:ident = $keysym:literal,)*) => {
        &[$($keysym),*]
    };
}

const DECLARED: &[u32] = key_constants!(declared);

fn is_a_number(written: &str) -> bool {
    let key = written.rsplit('>').next().unwrap_or(written);
    key.starts_with("0x") || key.starts_with("U+")
}

#[test]
fn what_the_writer_writes_for_a_declared_key_reads_back_as_its_lower_case() {
    let six = [SHIFT, CONTROL, ALT, META, SUPER, HYPER];
    let mut read_back = 0;
    for keysym in DECLARED {
        for subset in 0..1_u32 << six.len() {
            let modifiers = six
                .iter()
                .enumerate()
                .filter(|(bit, _)| subset >> bit & 1 == 1)
                .fold(Modifiers::empty(), |all, (_, modifier)| all | *modifier);
            let written = name(key(*keysym), modifiers);
            let expected = (!is_a_number(&written))
                .then(|| (Xkb.key_to_lower(key(*keysym)), modifiers));
            assert_eq!(parse(&written), expected, "{written:?}");
            read_back += usize::from(expected.is_some());
        }
    }
    // Every declared key but `VoidSymbol`, which is written as a number.
    assert_eq!(read_back, (DECLARED.len() - 1) * 64);
}
