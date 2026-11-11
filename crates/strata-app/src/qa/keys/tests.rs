// SPDX-License-Identifier: MIT

use rune::{Key, Mods};
use strata_qa::Modifier;

use super::{NAMES, Named, event, mods, named, needs_shift};

fn plain(name: &str) -> Key {
    event(named(name).expect("a key"), None, Mods::default())
        .key
        .clone()
}

/// The named keys the slice's parity set presses (`docs/rune/slice/V7-rune-qa-backend.md`).
#[test]
fn the_parity_sets_named_keys_are_runes_keys() {
    for (name, expected) in [
        ("BackSpace", Key::Backspace),
        ("Delete", Key::Delete),
        ("Down", Key::Down),
        ("End", Key::End),
        ("Escape", Key::Escape),
        ("F2", Key::Function(2)),
        ("F5", Key::Function(5)),
        ("F10", Key::Function(10)),
        ("Home", Key::Home),
        ("KP_Left", Key::Left),
        ("Left", Key::Left),
        ("Menu", Key::ContextMenu),
        ("Page_Down", Key::PageDown),
        ("Return", Key::Enter),
        ("Right", Key::Right),
        ("Tab", Key::Tab),
        ("Up", Key::Up),
        ("space", Key::Text(" ".to_owned())),
        ("period", Key::Text(".".to_owned())),
    ] {
        assert_eq!(plain(name), expected, "{name}");
    }
}

#[test]
fn a_character_is_the_key_that_types_it() {
    for character in "123\\acfhijklmnoqtvz".chars() {
        let name = character.to_string();
        assert_eq!(plain(&name), Key::Text(name.clone()), "{name}");
        assert!(!needs_shift(character), "{name} needs no Shift");
    }
    for character in "AZ?{~".chars() {
        assert!(needs_shift(character), "{character} needs Shift");
    }
}

#[test]
fn names_that_are_no_key_are_refused() {
    for name in ["", "F0", "F25", "Hyper_L", "return"] {
        assert!(named(name).is_err(), "{name:?} is no key");
    }
}

#[test]
fn a_character_with_ctrl_held_is_a_command_as_a_window_sends_it() {
    let ctrl = mods(&[Modifier::Ctrl], false);
    assert!(ctrl.ctrl && ctrl.cmd, "Ctrl is the primary modifier too");
    let pressed = event(Named::Character('A'), None, ctrl);
    assert_eq!(pressed.key, Key::Command('a'));
    // Ctrl+Space stays text, the one text a window sends with the primary modifier.
    let space = event(Named::Character(' '), None, ctrl);
    assert_eq!(space.key, Key::Text(" ".to_owned()));
    let enter = event(Named::Enter, None, ctrl);
    assert_eq!(enter.key, Key::Enter);
}

#[test]
fn a_command_on_a_key_that_types_no_ascii_names_the_us_key() {
    let ctrl = mods(&[Modifier::Ctrl], false);
    let pressed = event(Named::Character('м'), Some('v'), ctrl);
    assert_eq!(pressed.key, Key::Command('v'));
    // Without Ctrl the layout's own character is typed.
    let typed = event(Named::Character('м'), Some('v'), Mods::default());
    assert_eq!(typed.key, Key::Text("м".to_owned()));
}

#[test]
fn the_right_alt_key_is_alt_unless_the_layout_shifts_levels_with_it() {
    assert!(mods(&[Modifier::AltGr], false).alt);
    assert!(!mods(&[Modifier::AltGr], true).alt);
    assert!(mods(&[Modifier::Alt], true).alt);
    assert!(mods(&[Modifier::Shift], false).shift);
}

#[test]
fn each_name_is_listed_once() {
    let mut names: Vec<&str> = NAMES.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    let listed = names.len();
    names.dedup();
    assert_eq!(names.len(), listed);
}
