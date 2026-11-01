// SPDX-License-Identifier: MIT

use super::*;

type LabelCase = (&'static str, KeyPress, Binding);

fn cyrillic_label_cases() -> Vec<LabelCase> {
    vec![
        (
            "us,ru Ctrl+С",
            press(slot(&[US, RU], 1, 54, 0), typed(Key::Cyrillic_es, CTRL)),
            Binding::ctrl(Key::c),
        ),
        (
            "ru-only Ctrl+С uses US-QWERTY",
            press(slot(&[RU], 0, 54, 0), typed(Key::Cyrillic_es, CTRL)),
            Binding::ctrl(Key::c),
        ),
        (
            "ru-only Ctrl+Л is Ctrl+K",
            press(slot(&[RU], 0, 45, 0), typed(Key::Cyrillic_el, CTRL)),
            Binding::ctrl(Key::k),
        ),
        (
            "us,ru о is j",
            press(slot(&[US, RU], 1, 44, 0), typed(Key::Cyrillic_o, NONE)),
            Binding::plain(Key::j),
        ),
        (
            "us,ru Shift+О is Shift+j",
            press(
                slot(&[US, RU], 1, 44, 1),
                Typed {
                    typed: Key::Cyrillic_O,
                    matched: (Key::Cyrillic_o, SHIFT),
                    state: SHIFT,
                },
            ),
            Binding::shift(Key::j),
        ),
        (
            "uk,us і is s",
            press(slot(&[UK, US], 0, 39, 0), typed(Key::Ukrainian_i, CTRL)),
            Binding::ctrl(Key::s),
        ),
    ]
}

fn greek_and_hebrew_label_cases() -> Vec<LabelCase> {
    vec![
        (
            "el,us ψ is c",
            press(slot(&[EL, US], 0, 54, 0), typed(Key::Greek_psi, CTRL)),
            Binding::ctrl(Key::c),
        ),
        (
            "el,us ς is w",
            press(
                slot(&[EL, US], 0, 25, 0),
                typed(Key::Greek_finalsmallsigma, NONE),
            ),
            Binding::plain(Key::w),
        ),
        (
            "he,us ב is c",
            press(slot(&[HE, US], 0, 54, 0), typed(Key::hebrew_bet, CTRL)),
            Binding::ctrl(Key::c),
        ),
        (
            "he Shift types Latin C",
            press(
                slot(&[HE, US], 0, 54, 1),
                Typed {
                    typed: Key::C,
                    matched: (Key::c, CTRL_SHIFT),
                    state: CTRL_SHIFT,
                },
            ),
            Binding::ctrl_shift(Key::c),
        ),
    ]
}

fn latin_layout_label_cases() -> Vec<LabelCase> {
    vec![
        (
            "us Ctrl+C",
            press(slot(&[US], 0, 54, 0), typed(Key::c, CTRL)),
            Binding::ctrl(Key::c),
        ),
        (
            "fr AZERTY A key",
            press(slot(&[FR], 0, 24, 0), typed(Key::a, CTRL)),
            Binding::ctrl(Key::a),
        ),
        (
            "de QWERTZ Z key",
            press(slot(&[DE], 0, 29, 0), typed(Key::z, CTRL)),
            Binding::ctrl(Key::z),
        ),
        (
            "Dvorak C position types j",
            press(slot(&[DVORAK], 0, 54, 0), typed(Key::j, NONE)),
            Binding::plain(Key::j),
        ),
    ]
}

#[test]
fn letters_follow_the_label_then_the_physical_key() {
    let cases = latin_layout_label_cases()
        .into_iter()
        .chain(cyrillic_label_cases())
        .chain(greek_and_hebrew_label_cases());
    for (name, press, binding) in cases {
        assert!(press.is(binding), "{name}: {press:?}");
    }
    let de_ru = press(slot(&[DE, RU], 1, 29, 0), typed(Key::Cyrillic_en, CTRL));
    assert!(
        de_ru.is(Binding::ctrl(Key::z)) && !de_ru.is(Binding::ctrl(Key::y)),
        "a configured Latin layout wins over the US table: {de_ru:?}"
    );
}

#[test]
fn caps_lock_never_changes_a_shortcut_or_the_fallback_level() {
    let cases = [
        (
            "us Caps Lock",
            press(
                slot(&[US], 0, 54, 1),
                Typed {
                    typed: Key::C,
                    matched: (Key::c, CTRL),
                    state: CTRL.union(CAPS),
                },
            ),
            Binding::ctrl(Key::c),
        ),
        (
            "ru Caps Lock raises the level, not Shift",
            press(
                slot(&[US, RU], 1, 54, 1),
                Typed {
                    typed: Key::Cyrillic_ES,
                    matched: (Key::Cyrillic_es, CTRL),
                    state: CTRL.union(CAPS),
                },
            ),
            Binding::ctrl(Key::c),
        ),
        (
            "ru Caps Lock with Shift",
            press(
                slot(&[US, RU], 1, 54, 0),
                Typed {
                    typed: Key::Cyrillic_es,
                    matched: (Key::Cyrillic_es, CTRL_SHIFT),
                    state: CTRL_SHIFT.union(CAPS),
                },
            ),
            Binding::ctrl_shift(Key::c),
        ),
    ];
    for (name, press, binding) in cases {
        assert!(press.is(binding), "{name}: {press:?}");
    }
}

#[test]
fn the_active_layouts_ascii_character_wins_over_the_fallback() {
    // On Russian the key at the US slash position types `.`, and `ю` sits on the US `.` key.
    let period = press(slot(&[US, RU], 1, 61, 0), typed(Key::period, CTRL));
    let yu = press(slot(&[US, RU], 1, 60, 0), typed(Key::Cyrillic_yu, CTRL));
    let greek_q = press(slot(&[EL, US], 0, 24, 0), typed(Key::semicolon, CTRL));
    assert!(period.is(Binding::ctrl(Key::period)) && !period.is(Binding::ctrl(Key::slash)));
    assert!(yu.is(Binding::ctrl(Key::period)));
    assert!(greek_q.is(Binding::ctrl(Key::semicolon)) && !greek_q.is(Binding::ctrl(Key::q)));
}

#[test]
fn symbols_ignore_the_shift_that_types_them() {
    let ru_shift_yu = press(
        slot(&[US, RU], 1, 60, 1),
        Typed {
            typed: Key::Cyrillic_YU,
            matched: (Key::Cyrillic_yu, CTRL_SHIFT),
            state: CTRL_SHIFT,
        },
    );
    let us_greater = press(
        slot(&[US], 0, 60, 1),
        Typed {
            typed: Key::greater,
            matched: (Key::greater, CTRL),
            state: CTRL_SHIFT,
        },
    );
    let azerty_period = press(
        slot(&[FR], 0, 59, 1),
        Typed {
            typed: Key::period,
            matched: (Key::period, CTRL),
            state: CTRL_SHIFT,
        },
    );
    assert!(ru_shift_yu.is(Binding::ctrl(Key::greater)));
    assert!(us_greater.is(Binding::ctrl(Key::greater)));
    assert!(azerty_period.is(Binding::ctrl(Key::period)));
}

#[test]
fn altgr_characters_never_fall_back() {
    let superscript = press(slot(&[FR], 0, 10, 2), typed(Key::onesuperior, NONE));
    let macron = press(slot(&[US], 0, 10, 2), typed(Key::dead_macron, NONE));
    // A dead key types nothing by itself.
    for (press, text) in [(superscript, Some('¹')), (macron, None)] {
        assert!(!press.used_fallback(), "{press:?}");
        assert_eq!(press.text(), text, "{press:?}");
    }
}

#[test]
fn typing_receives_the_character_the_layout_produced() {
    let es = press(slot(&[US, RU], 1, 54, 0), typed(Key::Cyrillic_es, NONE));
    assert_eq!(es.text(), Some('с'));
    assert!(es.is(Binding::plain(Key::c)));
    let caps_es = press(
        slot(&[US, RU], 1, 54, 1),
        Typed {
            typed: Key::Cyrillic_ES,
            matched: (Key::Cyrillic_es, NONE),
            state: CAPS,
        },
    );
    assert_eq!(caps_es.text(), Some('С'));
    let ctrl_es = press(slot(&[US, RU], 1, 54, 0), typed(Key::Cyrillic_es, CTRL));
    assert_eq!(ctrl_es.text(), None);
}
