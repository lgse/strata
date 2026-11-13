// SPDX-License-Identifier: MIT

//! The press builder against the catalog, in the cases GTK's layout tests hold the GDK
//! reader to (`src/ui/keys/tests/letters.rs`, `digits.rs`, `modifiers.rs`): each is what
//! Rune's window host delivers for that key under that layout, with where the key is and
//! what it types with nothing held.

use rune::{Key as Logical, KeyEvent, Modifier, Mods, PhysicalKeyCode as At};
use strata_desktop::keys::{
    Binding, Key, KeyPress, Map, Modifiers, SHORTCUT_MODIFIERS,
    bindings::{
        ARROW_SCOPE, CONTEXT_MENU, COPY, FILTER_SLASH, FOCUS_PREVIOUS, HIDDEN_FILES, NEW_FOLDER,
        PROPERTIES, REFRESH, SELECT_ALL, SETTINGS, TAB_NUMBERS, TEXT_LARGER, UNDO, VIEW_COLUMNS,
    },
};

use super::{Origin, read};

const NONE: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};
const SHIFT: Mods = Mods {
    shift: true,
    ..NONE
};
/// Ctrl as Rune's window host reports it off macOS: the primary modifier too.
const CTRL: Mods = Mods {
    cmd: true,
    ctrl: true,
    ..NONE
};
const CTRL_SHIFT: Mods = Mods {
    shift: true,
    ..CTRL
};

/// A key a window delivered: where it is, and what it types with nothing held.
fn at(physical: At, base: char) -> Origin {
    Origin {
        physical: Some(physical),
        base: Some(base),
    }
}

/// A key with no physical origin: the QA target's, and a headless test's.
const NOWHERE: Origin = Origin {
    physical: None,
    base: None,
};

fn typed(character: char, mods: Mods, origin: Origin) -> KeyPress {
    crate::startup::install_key_library().expect("the key library");
    let key = Logical::Text(character.to_string());
    read(&KeyEvent { key, mods }, origin)
}

/// A character with Ctrl held, as Rune's host names it: lowered.
fn chord(character: char, mods: Mods, origin: Origin) -> KeyPress {
    crate::startup::install_key_library().expect("the key library");
    let key = Logical::Command(character);
    read(&KeyEvent { key, mods }, origin)
}

fn named(key: Logical, mods: Mods) -> KeyPress {
    crate::startup::install_key_library().expect("the key library");
    read(&KeyEvent { key, mods }, NOWHERE)
}

#[test]
fn letters_follow_the_label_then_the_us_key_at_the_position() {
    let cases = [
        (
            "us Ctrl+C",
            chord('c', CTRL, at(At::KeyC, 'c')),
            COPY.primary(),
        ),
        (
            "fr AZERTY: the A key is where Q is",
            chord('a', CTRL, at(At::KeyQ, 'a')),
            SELECT_ALL.primary(),
        ),
        (
            "de QWERTZ: the Z key is where Y is",
            chord('z', CTRL, at(At::KeyY, 'z')),
            UNDO.primary(),
        ),
        (
            "Dvorak: the C position types j",
            typed('j', NONE, at(At::KeyC, 'j')),
            Some(Binding::plain(Key::j)),
        ),
        (
            "ru: о is j",
            typed('о', NONE, at(At::KeyJ, 'о')),
            Some(Binding::plain(Key::j)),
        ),
        (
            "ru: Shift+О is Shift+j",
            typed('О', SHIFT, at(At::KeyJ, 'о')),
            Some(Binding::shift(Key::j)),
        ),
        (
            "el: ς is w",
            typed('ς', NONE, at(At::KeyW, 'ς')),
            Some(Binding::plain(Key::w)),
        ),
        (
            "ru: the key at the US slash types a period, which wins",
            chord('.', CTRL, at(At::Slash, '.')),
            Some(Binding::ctrl(Key::period)),
        ),
        (
            "el: the key at the US q types a semicolon, which wins",
            chord(';', CTRL, at(At::KeyQ, ';')),
            Some(Binding::ctrl(Key::semicolon)),
        ),
    ];
    for (case, press, binding) in cases {
        let binding = binding.expect("a shipped binding");
        assert!(press.is(binding), "{case}: {press:?}");
    }
}

/// Off macOS Rune's host has already put the US key in a non-ASCII character's place when
/// Ctrl is held. The base says what the key types, so the press reads as GDK's does.
#[test]
fn a_chord_on_a_layout_without_ascii_is_the_us_key_and_says_it_fell_back() {
    let es = chord('c', CTRL, at(At::KeyC, 'с'));
    assert!(es.is(COPY) && es.used_fallback(), "{es:?}");
    let yu = chord('.', CTRL, at(At::Period, 'ю'));
    assert!(yu.is(HIDDEN_FILES.in_map(Map::Default)), "{yu:?}");
    let hebrew = chord('c', CTRL_SHIFT, at(At::KeyC, 'ב'));
    assert!(hebrew.is(Binding::ctrl_shift(Key::c)), "{hebrew:?}");
    let latin = chord('c', CTRL, at(At::KeyC, 'c'));
    assert!(!latin.used_fallback(), "{latin:?}");
}

/// A letter beyond ASCII is the key GDK has for it, lowered, with the Shift that typed its
/// capital kept: what the GTK application would record for the same press.
#[test]
fn a_letter_beyond_ascii_is_recorded_as_gdk_records_it() {
    let cases = [
        (
            "ru: я with no key to fall back to",
            typed('я', NONE, NOWHERE),
            Binding::plain(Key::from_keysym(0x6d1)),
        ),
        (
            "ru: Shift+Я",
            typed('Я', SHIFT, NOWHERE),
            Binding::shift(Key::from_keysym(0x6d1)),
        ),
        (
            "É typed with no Shift, under Caps Lock, is é",
            typed('É', NONE, NOWHERE),
            Binding::plain(Key::from_keysym(0xe9)),
        ),
        (
            "pl: ą at a level above Shift's",
            typed('ą', NONE, at(At::KeyA, 'a')),
            Binding::plain(Key::from_keysym(0x1b1)),
        ),
    ];
    for (case, press, binding) in cases {
        assert_eq!(press.binding(), Some(binding), "{case}: {press:?}");
        assert!(press.is(binding), "{case}: {press:?}");
    }
}

/// The default in force for the eighth key case (`docs/rune/parity/owner.md`, B3): where
/// only the level typed is not ASCII, Rune has dropped the character, the base says nothing
/// was replaced, and the press is the US key's. GTK reads Ctrl+`|` and no shortcut.
#[test]
fn a_chord_whose_level_alone_is_not_ascii_reads_as_the_us_key_rune_names() {
    let azerty_mu = chord('\\', CTRL_SHIFT, at(At::Backslash, '*'));
    assert!(azerty_mu.is(ARROW_SCOPE), "{azerty_mu:?}");
    let polish_ogonek = chord('a', CTRL, at(At::KeyA, 'a'));
    assert!(polish_ogonek.is(SELECT_ALL), "{polish_ogonek:?}");
}

#[test]
fn caps_lock_never_changes_a_shortcut() {
    let capital = typed('C', NONE, at(At::KeyC, 'c'));
    assert!(capital.is(Binding::plain(Key::c)), "{capital:?}");
    assert!(!capital.is(Binding::shift(Key::c)), "{capital:?}");
    assert_eq!(capital.text(), Some('C'));
    let cyrillic = typed('С', NONE, at(At::KeyC, 'с'));
    assert!(cyrillic.is(Binding::plain(Key::c)), "{cyrillic:?}");
    assert!(!cyrillic.is(Binding::shift(Key::c)), "{cyrillic:?}");
    assert_eq!(cyrillic.text(), Some('С'));
    let cyrillic_shifted = typed('С', SHIFT, at(At::KeyC, 'с'));
    assert!(
        cyrillic_shifted.is(Binding::shift(Key::c)),
        "{cyrillic_shifted:?}"
    );
    assert!(
        !cyrillic_shifted.is(Binding::plain(Key::c)),
        "{cyrillic_shifted:?}"
    );
    let cyrillic_chord = chord('c', CTRL_SHIFT, at(At::KeyC, 'с'));
    assert!(
        cyrillic_chord.is(Binding::ctrl_shift(Key::c)),
        "{cyrillic_chord:?}"
    );
    assert!(cyrillic_chord.used_fallback(), "{cyrillic_chord:?}");
    let shifted = typed('C', SHIFT, at(At::KeyC, 'c'));
    assert!(shifted.is(Binding::shift(Key::c)), "{shifted:?}");
}

#[test]
fn a_symbol_ignores_the_shift_that_types_it() {
    let cases = [
        (
            "us Ctrl+Shift+period is Ctrl+>",
            chord('>', CTRL_SHIFT, at(At::Period, '.')),
            Binding::ctrl(Key::greater),
        ),
        (
            "fr AZERTY types a period with Shift",
            chord('.', CTRL_SHIFT, at(At::Comma, ';')),
            Binding::ctrl(Key::period),
        ),
        (
            "de Shift+7 is a slash",
            typed('/', SHIFT, at(At::Digit7, '7')),
            Binding::plain(Key::slash),
        ),
    ];
    for (case, press, binding) in cases {
        assert!(press.is(binding), "{case}: {press:?}");
    }
    let slash = typed('/', SHIFT, at(At::Digit7, '7'));
    assert!(slash.is(FILTER_SLASH) && slash.text() == Some('/'));
}

#[test]
fn the_digit_row_counts_by_position_with_or_without_shift() {
    let azerty = chord('&', CTRL, at(At::Digit1, '&'));
    assert!(azerty.is(VIEW_COLUMNS), "{azerty:?}");
    let cases = [
        ("us", chord('!', CTRL_SHIFT, at(At::Digit1, '1'))),
        ("fr AZERTY", chord('1', CTRL_SHIFT, at(At::Digit1, '&'))),
        ("ru", chord('"', CTRL_SHIFT, at(At::Digit2, '2'))),
    ];
    let digits: Vec<_> = cases
        .iter()
        .map(|(_, press)| press.digit_in(TAB_NUMBERS))
        .collect();
    assert_eq!(digits, [Some(1), Some(1), Some(2)]);
    let us = &cases[0].1;
    assert!(!us.is(VIEW_COLUMNS), "Ctrl+Shift+1 is not Ctrl+1: {us:?}");
}

/// AltGr is no modifier Rune reports. A character that is neither the key's base nor typed
/// with Shift is above level 1: text, with no fallback, and a digit only if it types one.
#[test]
fn a_character_above_shifts_level_is_text_and_never_falls_back() {
    let brace = typed('{', NONE, at(At::Digit7, '7'));
    assert!(brace.is(Binding::plain(Key::braceleft)), "{brace:?}");
    assert!(!brace.is(Binding::plain(Key::_7)), "{brace:?}");

    let ogonek = typed('ą', NONE, at(At::KeyA, 'a'));
    assert!(!ogonek.used_fallback() && !ogonek.is(Binding::plain(Key::a)));
    assert_eq!(ogonek.text(), Some('ą'));

    let superscript = typed('¹', NONE, at(At::Digit1, '&'));
    assert!(!superscript.used_fallback(), "{superscript:?}");
    assert!(!superscript.is(Binding::plain(Key::_1)), "{superscript:?}");

    let lithuanian = typed('1', NONE, at(At::Digit1, 'ą'));
    assert!(lithuanian.is(Binding::plain(Key::_1)), "{lithuanian:?}");
}

#[test]
fn a_keypad_key_is_the_keypad_key_gdk_reports() {
    let five = typed('5', NONE, at(At::Numpad5, '5'));
    assert!(five.is(Binding::plain(Key::_5)) && five.text() == Some('5'));
    let plus = chord('+', CTRL, at(At::NumpadAdd, '+'));
    assert!(plus.is(TEXT_LARGER), "{plus:?}");
    let decimal = chord('.', CTRL, at(At::NumpadDecimal, '.'));
    assert!(
        !decimal.is(HIDDEN_FILES.in_map(Map::Default)),
        "{decimal:?}"
    );
    let decimal = typed('.', NONE, at(At::NumpadDecimal, '.'));
    assert_eq!(decimal.text(), Some('.'));
    // Where the layout makes the same key type a comma, it is the keypad's separator.
    let separator = chord(',', CTRL, at(At::NumpadDecimal, ','));
    assert!(!separator.is(SETTINGS), "{separator:?}");
    let separator = typed(',', NONE, at(At::NumpadDecimal, ','));
    assert_eq!(separator.text(), Some(','));
}

#[test]
fn a_named_key_is_itself_with_what_is_held() {
    let all = Mods {
        shift: true,
        alt: true,
        logo: true,
        ..CTRL
    };
    let alt = Mods { alt: true, ..NONE };
    let logo = Mods { logo: true, ..NONE };
    let f5 = named(Logical::Function(5), all);
    assert!(f5.is(REFRESH.ignoring(SHORTCUT_MODIFIERS)) && !f5.is(REFRESH));
    assert!(named(Logical::Tab, SHIFT).is(FOCUS_PREVIOUS));
    assert!(named(Logical::Enter, alt).is(PROPERTIES));
    assert!(named(Logical::ContextMenu, NONE).is(CONTEXT_MENU));
    assert!(named(Logical::Function(10), SHIFT).is(CONTEXT_MENU));
    // Rune's one flag for Super, Hyper and Meta is read as Super, which no binding ignores.
    assert!(!named(Logical::ContextMenu, logo).is(CONTEXT_MENU));
    let unknown = named(Logical::Function(25), NONE);
    assert_eq!(unknown.binding(), None, "no key a keymap could hold");
}

/// The QA target's presses and a headless test's have no position: the character is read as
/// the US key that types it. A plain letter of another alphabet then has no fallback, which
/// is Rune packet entry 97.
#[test]
fn a_press_with_no_origin_reads_as_the_us_key_that_types_it() {
    assert!(chord('c', CTRL, NOWHERE).is(COPY));
    assert!(chord('n', CTRL_SHIFT, NOWHERE).is(NEW_FOLDER));
    assert_eq!(
        chord('!', CTRL_SHIFT, NOWHERE).digit_in(TAB_NUMBERS),
        Some(1)
    );
    // A test's chord with Ctrl alone held, as `shell/tests.rs` presses one.
    let ctrl = Mods { ctrl: true, ..NONE };
    assert!(typed('h', ctrl, NOWHERE).is(HIDDEN_FILES.in_map(Map::Default)));
    let cyrillic = typed('с', NONE, NOWHERE);
    assert!(!cyrillic.is(Binding::plain(Key::c)) && cyrillic.text() == Some('с'));
}

#[test]
fn composed_text_of_several_characters_is_no_key_and_no_character() {
    crate::startup::install_key_library().expect("the key library");
    let key = Logical::Text("ab".to_owned());
    let press = read(&KeyEvent { key, mods: NONE }, NOWHERE);
    assert_eq!((press.text(), press.binding()), (None, None));
}

/// GDK reports the modifiers held before the event, and Rune those after it.
#[test]
fn a_modifiers_own_press_is_held_without_it_and_its_release_with_it() {
    let shift = |pressed, mods| {
        let key = Logical::Modifier {
            modifier: Modifier::ShiftLeft,
            pressed,
        };
        named(key, mods)
    };
    let down = shift(true, SHIFT);
    assert!(down.is_modifier());
    assert_eq!(down.modifier_key(), Modifiers::SHIFT_MASK);
    assert_eq!(down.modifiers(), Modifiers::empty());
    let up = shift(false, NONE);
    assert_eq!(up.modifiers(), Modifiers::SHIFT_MASK);
}
