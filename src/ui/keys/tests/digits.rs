// SPDX-License-Identifier: MIT

use super::*;

fn shift_on_the_digit_row_follows_the_level(tab_numbers: Digits) {
    let azerty_ctrl_1 = press(slot(&[FR], 0, 10, 0), typed(Key::ampersand, CTRL));
    let azerty_ctrl_shift_1 = press(
        slot(&[FR], 0, 10, 1),
        Typed {
            typed: Key::_1,
            matched: (Key::_1, CTRL),
            state: CTRL_SHIFT,
        },
    );
    let qwertz_ctrl_shift_2 = press(
        slot(&[DE], 0, 11, 1),
        Typed {
            typed: Key::quotedbl,
            matched: (Key::quotedbl, CTRL),
            state: CTRL_SHIFT,
        },
    );
    let us_ctrl_shift_1 = press(
        slot(&[US], 0, 10, 1),
        Typed {
            typed: Key::exclam,
            matched: (Key::exclam, CTRL),
            state: CTRL_SHIFT,
        },
    );
    let cases = [
        (&azerty_ctrl_1, Binding::ctrl(Key::_1), true),
        (&azerty_ctrl_1, Binding::ctrl_shift(Key::_1), false),
        (&azerty_ctrl_shift_1, Binding::ctrl_shift(Key::_1), true),
        (&azerty_ctrl_shift_1, Binding::ctrl(Key::_1), false),
        (&qwertz_ctrl_shift_2, Binding::ctrl_shift(Key::_2), true),
        (&us_ctrl_shift_1, Binding::ctrl_shift(Key::_1), true),
        (&us_ctrl_shift_1, Binding::ctrl(Key::_1), false),
    ];
    for (press, binding, matches) in cases {
        assert_eq!(press.is(binding), matches, "{press:?} against {binding:?}");
    }
    assert_eq!(azerty_ctrl_shift_1.digit_in(tab_numbers), Some(1));
    assert_eq!(azerty_ctrl_1.digit_in(tab_numbers), None);
}

/// A chord's digit step accepts the Shift AZERTY types digits with.
fn chord_steps_take_the_digit_a_symbol_key_shares(chord_step: Digits) {
    let azerty_shift_6 = press(
        slot(&[FR], 0, 15, 1),
        Typed {
            typed: Key::_6,
            matched: (Key::_6, NONE),
            state: SHIFT,
        },
    );
    assert_eq!(azerty_shift_6.digit_in(chord_step), Some(6));
    // A top-row + or − is both: g and Ctrl read the symbol first, while t and ;
    // have no symbol step and take the digit.
    let azerty_minus = press(slot(&[FR], 0, 15, 0), typed(Key::minus, NONE));
    let cz_plus = press(slot(&[CZ], 0, 10, 0), typed(Key::plus, NONE));
    for (press, symbol, digit) in [
        (azerty_minus, Binding::plain(Key::minus), 6),
        (cz_plus, Binding::plain(Key::plus), 1),
    ] {
        assert!(press.is(symbol), "{press:?}");
        assert_eq!(press.digit_in(chord_step), Some(digit), "{press:?}");
    }
}

/// Hungarian prints 0 left of 1 and Ö on the tenth key; both read as 0.
fn hungarian_zero_sits_left_of_one() {
    let hu_zero = press(slot(&[HU], 0, 49, 0), typed(Key::_0, CTRL));
    let hu_tenth = press(slot(&[HU], 0, 19, 0), typed(Key::odiaeresis, CTRL));
    assert!(hu_zero.is(Binding::ctrl(Key::_0)), "{hu_zero:?}");
    assert!(hu_tenth.is(Binding::ctrl(Key::_0)), "{hu_tenth:?}");
    let hu_section = press(
        slot(&[HU], 0, 49, 1),
        Typed {
            typed: Key::section,
            matched: (Key::section, CTRL),
            state: CTRL_SHIFT,
        },
    );
    assert!(!hu_section.is(Binding::ctrl_shift(Key::_0)));
}

/// GDK reports no AltGr modifier: above level 1 the row counts only a digit
/// it types, as Lithuanian does, never an AltGr symbol.
fn altgr_rows_count_only_the_digits_they_type(tab_numbers: Digits, chord_step: Digits) {
    let lt_ctrl_1 = press(slot(&[LT], 0, 10, 2), typed(Key::_1, CTRL));
    assert!(lt_ctrl_1.is(Binding::ctrl(Key::_1)));
    let azerty_ctrl_superscript = press(slot(&[FR], 0, 10, 2), typed(Key::onesuperior, CTRL));
    assert!(!azerty_ctrl_superscript.is(Binding::ctrl(Key::_1)));
    let azerty_tilde = press(slot(&[FR], 0, 11, 2), typed(Key::asciitilde, NONE));
    assert_eq!(azerty_tilde.digit_in(chord_step), None);
    // German Mac types \ with Shift+AltGr+7: Ctrl+\, not tab 7.
    let de_mac_backslash = press(
        slot(&[DE], 0, 16, 3),
        Typed {
            typed: Key::backslash,
            matched: (Key::backslash, CTRL),
            state: CTRL_SHIFT,
        },
    );
    assert_eq!(de_mac_backslash.digit_in(tab_numbers), None);
    assert!(de_mac_backslash.is(Binding::ctrl(Key::backslash)));
}

#[test]
fn digit_row_keys_are_digits_by_position() {
    let tab_numbers = Digits::new(CTRL_SHIFT, 9, true);
    let chord_step = Digits::after(Binding::plain(Key::g).shown(), 9, false);
    shift_on_the_digit_row_follows_the_level(tab_numbers);
    chord_steps_take_the_digit_a_symbol_key_shares(chord_step);
    hungarian_zero_sits_left_of_one();
    altgr_rows_count_only_the_digits_they_type(tab_numbers, chord_step);
}

/// A US press whose Shift, if held, typed the character.
fn us(keycode: u32, level: u32, key: Key, state: Modifiers) -> KeyPress {
    press(
        slot(&[US], 0, keycode, level),
        Typed {
            typed: key,
            matched: (key, state - SHIFT),
            state,
        },
    )
}

fn a_capture_records_the_binding_dispatch_reads() {
    let cases = [
        (
            us(12, 1, Key::numbersign, CTRL_SHIFT),
            Binding::ctrl_shift(Key::_3),
        ),
        (
            press(
                slot(&[FR], 0, 10, 1),
                Typed {
                    typed: Key::_1,
                    matched: (Key::_1, CTRL),
                    state: CTRL_SHIFT,
                },
            ),
            Binding::ctrl_shift(Key::_1),
        ),
        (
            press(
                slot(&[DE], 0, 11, 1),
                Typed {
                    typed: Key::quotedbl,
                    matched: (Key::quotedbl, CTRL),
                    state: CTRL_SHIFT,
                },
            ),
            Binding::ctrl_shift(Key::_2),
        ),
        (
            us(12, 1, Key::numbersign, SHIFT),
            Binding::plain(Key::numbersign),
        ),
        (us(12, 0, Key::_3, CTRL), Binding::ctrl(Key::_3)),
        (
            press(
                slot(&[DE], 0, 16, 3),
                Typed {
                    typed: Key::backslash,
                    matched: (Key::backslash, CTRL),
                    state: CTRL_SHIFT,
                },
            ),
            Binding::ctrl(Key::backslash),
        ),
    ];
    for (pressed, recorded) in cases {
        assert_eq!(pressed.binding(), Some(recorded), "{pressed:?}");
        assert!(pressed.is(recorded), "{pressed:?}");
    }
    assert_eq!(
        us(12, 1, Key::numbersign, CTRL_SHIFT).digit_in(Digits::new(CTRL_SHIFT, 9, true)),
        Some(3),
        "tab numbers read the key the capture records"
    );
}

/// AZERTY's Ctrl+& is also Ctrl+1, which a capture must check as well.
fn a_capture_records_the_digit_position_beside_the_symbol() {
    let azerty_ctrl_1 = press(slot(&[FR], 0, 10, 0), typed(Key::ampersand, CTRL));
    assert_eq!(azerty_ctrl_1.binding(), Some(Binding::ctrl(Key::ampersand)));
    assert_eq!(azerty_ctrl_1.digit_position(), Some(Binding::ctrl(Key::_1)));
    assert!(azerty_ctrl_1.is(Binding::ctrl(Key::_1)));
    let plain_digit = us(12, 0, Key::_3, NONE);
    assert_eq!(plain_digit.digit_position(), None);
}

/// AltGr and a layout switch start a press rather than make one, and a
/// key the keymap cannot load is never recorded.
fn modifier_and_void_keys_are_never_recorded() {
    for key in [Key::ISO_Level3_Shift, Key::ISO_Next_Group] {
        let held = press(slot(&[DE], 0, 108, 0), typed(key, CTRL));
        assert!(held.is_modifier(), "{key:?}");
    }
    assert_eq!(us(0, 0, Key::VoidSymbol, CTRL).binding(), None);
}

#[test]
fn a_captured_press_records_the_key_dispatch_reads() {
    a_capture_records_the_binding_dispatch_reads();
    a_capture_records_the_digit_position_beside_the_symbol();
    modifier_and_void_keys_are_never_recorded();
}

#[test]
fn keypad_digits_count_only_with_num_lock() {
    let digit = press(slot(&[US], 0, 87, 0), typed(Key::KP_1, CTRL));
    let end = press(slot(&[US], 0, 87, 0), typed(Key::KP_End, CTRL));
    assert!(digit.is(Binding::ctrl(Key::_1)));
    assert!(!end.is(Binding::ctrl(Key::_1)));
}
