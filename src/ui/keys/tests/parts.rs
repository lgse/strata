// SPDX-License-Identifier: MIT

use strata_desktop::keys::{KeyPressParts, Typed as Types};

use super::*;

const LOCK: Modifiers = Modifiers::LOCK_MASK;

fn parts(key: Key, modifiers: Modifiers, held: Modifiers, typed: Types) -> KeyPressParts {
    KeyPressParts {
        key,
        modifiers,
        held,
        typed,
        digit: None,
        fallback: false,
    }
}

#[test]
fn a_press_from_parts_is_the_press_a_gdk_event_gives() {
    let shifted_letter = Typed {
        typed: Key::C,
        matched: (Key::c, CTRL_SHIFT),
        state: CTRL_SHIFT,
    };
    let shifted_digit = Typed {
        typed: Key::exclam,
        matched: (Key::exclam, CTRL),
        state: CTRL_SHIFT,
    };
    let cases = [
        (
            "a letter with Ctrl",
            press(slot(&[US], 0, 54, 0), typed(Key::c, CTRL)),
            parts(Key::c, CTRL, CTRL, Types::Character('c')),
        ),
        (
            "an uppercase letter is lowered and keeps Shift",
            press(slot(&[US], 0, 54, 1), shifted_letter),
            parts(Key::C, CTRL_SHIFT, CTRL_SHIFT, Types::Character('C')),
        ),
        (
            "Caps Lock alone is no modifier",
            KeyPress::from_signal(Key::C, 0, LOCK),
            parts(Key::c, NONE, LOCK, Types::Character('C')),
        ),
        (
            "a layout without ASCII gives the fallback key and the character it typed",
            press(slot(&[RU, US], 0, 54, 0), typed(Key::Cyrillic_es, CTRL)),
            KeyPressParts {
                fallback: true,
                ..parts(Key::c, CTRL, CTRL, Types::Character('с'))
            },
        ),
        (
            "a symbol drops the Shift that typed it and keeps the digit position",
            press(slot(&[US], 0, 10, 1), shifted_digit),
            KeyPressParts {
                digit: Some(1),
                ..parts(Key::exclam, CTRL, CTRL_SHIFT, Types::Character('!'))
            },
        ),
        (
            "ISO_Left_Tab is Shift+Tab",
            KeyPress::from_signal(Key::ISO_Left_Tab, 0, SHIFT),
            parts(
                Key::ISO_Left_Tab,
                SHIFT,
                SHIFT,
                Types::Key(Key::ISO_Left_Tab),
            ),
        ),
        (
            "a keypad key folds to the main key",
            KeyPress::from_signal(Key::KP_Enter, 0, NONE),
            parts(Key::KP_Enter, NONE, NONE, Types::Key(Key::KP_Enter)),
        ),
        (
            "a keypad digit is a digit",
            KeyPress::from_signal(Key::KP_5, 0, NONE),
            KeyPressParts {
                digit: Some(5),
                ..parts(Key::KP_5, NONE, NONE, Types::Key(Key::KP_5))
            },
        ),
        (
            "a modifier key is a modifier",
            KeyPress::from_signal(Key::Shift_L, 0, NONE),
            parts(Key::Shift_L, NONE, NONE, Types::Key(Key::Shift_L)),
        ),
        (
            "a press that typed nothing",
            press(slot(&[US], 0, 54, 0), typed(Key::VoidSymbol, NONE)),
            parts(Key::VoidSymbol, NONE, NONE, Types::Nothing),
        ),
    ];
    for (case, from_gdk, parts) in cases {
        assert_eq!(KeyPress::from_parts(parts), from_gdk, "{case}");
    }
}
