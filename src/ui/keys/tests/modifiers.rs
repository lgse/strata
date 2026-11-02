// SPDX-License-Identifier: MIT

use super::super::SHORTCUT_MODIFIERS;
use super::*;

#[test]
fn modifiers_match_exactly_over_ctrl_alt_shift_and_super() {
    let cases = [
        (CTRL.union(Modifiers::ALT_MASK), false),
        (CTRL.union(Modifiers::SUPER_MASK), false),
        (CTRL.union(Modifiers::HYPER_MASK), true),
        (NONE, false),
    ];
    for (state, matches) in cases {
        let ctrl_c = press(slot(&[US], 0, 54, 0), typed(Key::c, state));
        assert_eq!(ctrl_c.is(Binding::ctrl(Key::c)), matches, "{ctrl_c:?}");
    }
}

#[test]
fn keypad_keys_act_as_the_main_keys() {
    let enter = press(slot(&[US], 0, 104, 0), typed(Key::KP_Enter, NONE));
    let up = press(slot(&[US], 0, 80, 0), typed(Key::KP_Up, SHIFT));
    let add = press(slot(&[US], 0, 86, 0), typed(Key::KP_Add, CTRL));
    let cases = [
        (&enter, Binding::plain(Key::Return)),
        (&up, Binding::shift(Key::Up)),
        (&add, Binding::ctrl(Key::plus)),
    ];
    for (press, binding) in cases {
        assert!(press.is(binding), "{press:?}");
    }
    assert_eq!(add.text(), None);
}

#[test]
fn ignored_modifiers_are_the_only_ones_a_binding_tolerates() {
    let escape = Binding::plain(Key::Escape).ignoring(SHORTCUT_MODIFIERS);
    let arrow = Binding::plain(Key::Up).ignoring(CTRL_SHIFT);
    let shift_escape = press(slot(&[US], 0, 9, 0), typed(Key::Escape, SHIFT));
    let ctrl_shift_up = press(slot(&[US], 0, 111, 0), typed(Key::Up, CTRL_SHIFT));
    let alt_up = press(slot(&[US], 0, 111, 0), typed(Key::Up, Modifiers::ALT_MASK));
    let cases = [
        (&shift_escape, escape, true),
        (&shift_escape, Binding::plain(Key::Escape), false),
        (&ctrl_shift_up, arrow, true),
        (&alt_up, arrow, false),
    ];
    for (press, binding, tolerated) in cases {
        assert_eq!(
            press.is(binding),
            tolerated,
            "{press:?} against {binding:?}"
        );
    }
}

#[test]
fn signals_without_an_event_read_as_us_presses() {
    let exclam = KeyPress::from_signal(Key::exclam, 0, CTRL_SHIFT);
    assert!(exclam.is(Binding::ctrl_shift(Key::_1)));
    assert!(exclam.is(Binding::ctrl(Key::exclam)));
    for letter in [Key::K, Key::k] {
        assert!(KeyPress::from_signal(letter, 0, CTRL_SHIFT).is(Binding::ctrl_shift(Key::k)));
    }
    assert!(
        KeyPress::from_signal(Key::G, 0, CAPS).is(Binding::plain(Key::g)),
        "Caps Lock alone types G"
    );
    let read_as = [
        (Key::ISO_Left_Tab, NONE, Binding::shift(Key::Tab)),
        (Key::question, SHIFT, Binding::plain(Key::question)),
    ];
    for (key, state, binding) in read_as {
        assert!(KeyPress::from_signal(key, 0, state).is(binding), "{key:?}");
    }
    assert!(KeyPress::from_signal(Key::Shift_L, 0, NONE).is_modifier());
    assert_eq!(
        KeyPress::from_signal(Key::Control_R, 0, SHIFT).modifier_key(),
        CTRL
    );
    let typed = KeyPress::from_signal(Key::Cyrillic_es, 0, NONE);
    assert_eq!(typed.text(), Some('с'));
    assert!(!typed.used_fallback());
}
