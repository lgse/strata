// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType as Modifiers};

use super::super::{Binding, Digits, Keys, Shown, digit_label, label_in, shown};

const CTRL: Modifiers = Modifiers::CONTROL_MASK;

fn english(keys: Keys) -> String {
    label_in("en", &keys)
}

#[test]
fn notation_orders_modifiers_and_spells_out_named_keys() {
    let cases = [
        (Keys::One(Binding::ctrl(Key::c).shown()), "Ctrl+C"),
        (
            Keys::One(Binding::ctrl_shift(Key::n).shown()),
            "Ctrl+Shift+N",
        ),
        (
            Keys::One(
                Binding::new(
                    Key::t,
                    Modifiers::SUPER_MASK | Modifiers::SHIFT_MASK | CTRL | Modifiers::ALT_MASK,
                )
                .shown(),
            ),
            "Ctrl+Alt+Shift+Super+T",
        ),
        (Keys::One(Binding::plain(Key::j).shown()), "j"),
        (Keys::One(Binding::shift(Key::j).shown()), "J"),
        (Keys::One(Binding::plain(Key::Page_Up).shown()), "Page Up"),
        (
            Keys::One(Binding::shift(Key::Delete).shown()),
            "Shift+Delete",
        ),
        (Keys::One(Binding::shift(Key::Tab).shown()), "Shift+Tab"),
        (Keys::One(Binding::alt(Key::Return).shown()), "Alt+Enter"),
        (Keys::One(Binding::plain(Key::Escape).shown()), "Escape"),
        (Keys::One(Binding::ctrl(Key::plus).shown()), "Ctrl++"),
        (
            Keys::One(Binding::ctrl(Key::minus).shown()),
            "Ctrl+\u{2212}",
        ),
        (Keys::One(Binding::ctrl(Key::backslash).shown()), "Ctrl+\\"),
        (Keys::One(Binding::plain(Key::F5).shown()), "F5"),
        (Keys::One(Binding::plain(Key::question).shown()), "?"),
        (Keys::One(Binding::plain(Key::EuroSign).shown()), "€"),
        (Keys::One(Binding::ctrl(Key::ae).shown()), "Ctrl+Æ"),
        (Keys::One(Binding::plain(Key::ae).shown()), "æ"),
        (
            Keys::One(Binding::ctrl(Key::dead_circumflex).shown()),
            "Ctrl+^",
        ),
        (Keys::One(Binding::plain(Key::dead_diaeresis).shown()), "¨"),
        (Keys::One(Binding::plain(Key::Multi_key).shown()), "Compose"),
    ];
    for (keys, expected) in cases {
        assert_eq!(english(keys), expected);
    }
}

#[test]
fn alternatives_chords_and_digit_ranges() {
    const RENAME: &[Shown] = shown![Binding::plain(Key::F2), Binding::ctrl(Key::r)];
    const ARROWS: &[Shown] = shown![Binding::ctrl_alt(Key::Left), Binding::ctrl_alt(Key::Right)];
    const SCROLL: &[Shown] = shown![Binding::ctrl(Key::u), Binding::ctrl(Key::d)];
    const GO_HOME: &[Shown] = shown![Binding::plain(Key::g), Binding::plain(Key::h)];
    const TOP: [Keys; 2] = [
        Keys::Chord(shown![Binding::plain(Key::g), Binding::plain(Key::g)]),
        Keys::One(Binding::plain(Key::Home).shown()),
    ];
    const TAB_NUMBERS: Digits = Digits::new(CTRL.union(Modifiers::SHIFT_MASK), 9, true);
    const TAB_CHORD_NUMBERS: Digits = Digits::after(Binding::plain(Key::t).shown(), 9, true);
    const FOLDER_NUMBERS: Digits = Digits::after_chord(
        Binding::plain(Key::semicolon).shown(),
        Binding::plain(Key::period).shown(),
        9,
        true,
    );
    let cases = [
        (Keys::Any(RENAME), "F2 / Ctrl+R"),
        (Keys::Any(ARROWS), "Ctrl+Alt+← / →"),
        (Keys::Any(SCROLL), "Ctrl+U / Ctrl+D"),
        (Keys::Chord(GO_HOME), "g h"),
        (Keys::Mixed(&TOP), "g g / Home"),
        (Keys::Digits(TAB_NUMBERS), "Ctrl+Shift+1\u{2013}9 / 0"),
        (Keys::Digits(TAB_CHORD_NUMBERS), "t 1\u{2013}9 / t 0"),
        (Keys::Digits(TAB_CHORD_NUMBERS.step()), "1\u{2013}9 / 0"),
        (Keys::Digits(TAB_NUMBERS.step()), "1\u{2013}9 / 0"),
        (
            Keys::Digits(Digits::after(Binding::plain(Key::g).shown(), 9, false)),
            "g 1\u{2013}9",
        ),
        (Keys::Digits(FOLDER_NUMBERS), "; . 1\u{2013}9 / ; . 0"),
        (Keys::Digits(FOLDER_NUMBERS.step()), "1\u{2013}9 / 0"),
    ];
    for (keys, expected) in cases {
        assert_eq!(english(keys), expected);
    }
    // Neither label is translated, so the locale other tests set cannot change them.
    assert_eq!(digit_label(TAB_CHORD_NUMBERS, 0), "t 0");
    assert_eq!(digit_label(FOLDER_NUMBERS, 3), "; . 3");
    assert_eq!(digit_label(TAB_NUMBERS.step(), 3), "3");
}

#[test]
fn key_names_follow_the_interface_language() {
    let cases = [
        ("de", Keys::One(Binding::ctrl(Key::c).shown()), "Strg+C"),
        ("de", Keys::One(Binding::plain(Key::Delete).shown()), "Entf"),
        (
            "de",
            Keys::One(Binding::ctrl_shift(Key::Page_Up).shown()),
            "Strg+Umschalt+Bild auf",
        ),
        (
            "fr",
            Keys::One(Binding::plain(Key::Delete).shown()),
            "Suppr",
        ),
        (
            "fr",
            Keys::One(Binding::shift(Key::Return).shown()),
            "Maj+Entrée",
        ),
        ("es", Keys::One(Binding::plain(Key::Delete).shown()), "Supr"),
        (
            "es",
            Keys::One(Binding::ctrl(Key::Multi_key).shown()),
            "Ctrl+Componer",
        ),
        ("ru", Keys::One(Binding::ctrl(Key::c).shown()), "Ctrl+C"),
        (
            "ja",
            Keys::One(Binding::plain(Key::Escape).shown()),
            "Escape",
        ),
    ];
    for (locale, keys, expected) in cases {
        assert_eq!(label_in(locale, &keys), expected, "{locale}");
    }
}
