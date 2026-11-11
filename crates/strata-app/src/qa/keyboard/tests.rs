// SPDX-License-Identifier: MIT

use rune::{Key, Mods};
use strata_qa::{Keymap, Modifier};

use super::Keyboard;

fn russian() -> Keymap {
    Keymap {
        layouts: "us,ru".to_owned(),
        variants: String::new(),
        group: 1,
        caps_lock: false,
    }
}

fn us() -> Keymap {
    Keymap {
        layouts: "us".to_owned(),
        variants: String::new(),
        group: 0,
        caps_lock: false,
    }
}

#[test]
fn a_chord_holds_its_modifiers_and_the_shift_its_key_needs() {
    let keyboard = Keyboard::default();
    let down = keyboard.press("Down", &[Modifier::Shift]).expect("a key");
    assert_eq!(down.key, Key::Down);
    assert!(down.mods.shift && !down.mods.ctrl);
    let select_all = keyboard.press("a", &[Modifier::Ctrl]).expect("a key");
    assert_eq!(select_all.key, Key::Command('a'));
    assert!(select_all.mods.ctrl && select_all.mods.cmd && !select_all.mods.shift);
    let question = keyboard.press("question", &[]).expect("a key");
    assert_eq!(question.key, Key::Text("?".to_owned()));
    assert!(question.mods.shift, "a US keyboard types ? with Shift");
    assert!(keyboard.press("NoSuchKey", &[]).is_err());
}

#[test]
fn held_modifiers_join_every_press_until_they_are_released() {
    let mut keyboard = Keyboard::default();
    keyboard.hold(Modifier::Ctrl);
    keyboard.hold(Modifier::Ctrl);
    assert!(keyboard.mods().ctrl);
    let pressed = keyboard.press("Down", &[Modifier::Shift]).expect("a key");
    assert!(pressed.mods.ctrl && pressed.mods.shift);
    keyboard.release(Modifier::Ctrl);
    assert_eq!(keyboard.mods(), Mods::default());
    let pressed = keyboard.press("Down", &[]).expect("a key");
    assert!(!pressed.mods.ctrl);
}

/// The `layout` step's mechanism for a host with no display: what the runner's `keymap` step
/// sets is what later key names are read under, by their place on a US keyboard.
#[test]
fn a_layout_changes_what_a_key_name_types_and_the_us_layout_restores_it() {
    let mut keyboard = Keyboard::default();
    keyboard.set_layout(&russian()).expect("the layout");
    let typed = keyboard.press("j", &[]).expect("a key");
    assert_eq!(typed.key, Key::Text("о".to_owned()));
    // Ctrl+V on a Russian layout is still the command on V, as in a Rune window.
    let paste = keyboard.press("v", &[Modifier::Ctrl]).expect("a key");
    assert_eq!(paste.key, Key::Command('v'));
    assert_eq!(
        keyboard.press("Return", &[]).expect("a key").key,
        Key::Enter
    );
    assert_eq!(keyboard.typed('j').key, Key::Text("о".to_owned()));
    // A character no US key types is typed as it is.
    assert_eq!(keyboard.typed('é').key, Key::Text("é".to_owned()));

    keyboard.set_layout(&us()).expect("the layout");
    let typed = keyboard.press("j", &[]).expect("a key");
    assert_eq!(typed.key, Key::Text("j".to_owned()));
}

#[test]
fn a_layout_that_does_not_compile_leaves_the_keyboard_as_it_was() {
    let mut keyboard = Keyboard::default();
    keyboard.set_layout(&russian()).expect("the layout");
    let unknown = Keymap {
        layouts: "no-such-layout".to_owned(),
        ..us()
    };
    assert!(keyboard.set_layout(&unknown).is_err());
    let typed = keyboard.press("j", &[]).expect("a key");
    assert_eq!(typed.key, Key::Text("о".to_owned()));
}
