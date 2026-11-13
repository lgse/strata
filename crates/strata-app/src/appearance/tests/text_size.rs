// SPDX-License-Identifier: MIT

//! The text size from the keyboard: each key steps or resets the saved size within the shared
//! rule, and the look follows it at once and from the saved size at start; the keys work while
//! a rename is open, and Ctrl+= and the Icons view's Ctrl+Alt+= each change only their own.

use rune::{Key, Mods};
use strata_kit::look::{Look, LookParams};

use super::{ICONS, MODULE, ctrl, settings_file};
use crate::{
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, fixture,
        view::{field, hosted_with, open, press},
    },
};

/// The size `settings.toml` holds, once the window has written it.
fn saved_size() -> Option<i64> {
    let saved = std::fs::read_to_string(settings_file()).ok()?;
    saved
        .parse::<toml::Table>()
        .ok()?
        .get("text_size")?
        .as_integer()
}

/// The interface's type size the look draws at `size` px.
fn drawn_at(size: u16) -> f32 {
    let params = LookParams {
        text_size: size,
        ..LookParams::default()
    };
    Look::fallback(true).with_params(&params).kit.ty.ui.size
}

/// Presses `key` with Ctrl, and waits for `size` to be saved and drawn.
fn step(host: &mut Offscreen, key: char, size: u16) {
    press(host, Key::Command(key), ctrl());
    let saved = host.run_until(PATIENCE, |_| saved_size() == Some(i64::from(size)));
    assert!(saved, "Ctrl+{key} to save {size}: {:?}", saved_size());
    assert_eq!(host.app().look.kit.ty.ui.size, drawn_at(size), "Ctrl+{key}");
}

/// The window draws at the saved size from the start; Ctrl+= and Ctrl+plus step it up one
/// pixel and stop at 48, Ctrl+minus steps it down, and Ctrl+0 goes back to 13.
#[test]
fn the_text_size_keys_step_the_saved_size_and_the_look_follows() {
    hosted_with(
        MODULE,
        "columns",
        "text_size = 47\n".to_owned(),
        "text_size::the_text_size_keys_step_the_saved_size_and_the_look_follows",
        |mut host| {
            assert_eq!(host.app().look.kit.ty.ui.size, drawn_at(47));
            assert_ne!(drawn_at(47), drawn_at(13));

            for (key, size) in [('=', 48), ('+', 48), ('-', 47), ('0', 13)] {
                step(&mut host, key, size);
            }
        },
    );
}

/// Ctrl+= is the text size and Ctrl+Alt+= the Icons view's tile size, at the same stage:
/// each key changes its own size and leaves the other.
#[test]
fn ctrl_equal_and_ctrl_alt_equal_each_change_only_their_own_size() {
    hosted_with(
        MODULE,
        "icons",
        String::new(),
        "text_size::ctrl_equal_and_ctrl_alt_equal_each_change_only_their_own_size",
        |mut host| {
            let folder = fixture();
            open(&mut host, ICONS, folder.path());
            let tiles = crate::icons::size::shown(host.app());
            let text = shell::preferences(host.app()).text_size;

            let ctrl_alt = Mods {
                alt: true,
                ..ctrl()
            };
            press(&mut host, Key::Command('='), ctrl_alt);
            assert!(crate::icons::size::shown(host.app()) > tiles, "Ctrl+Alt+=");
            assert_eq!(shell::preferences(host.app()).text_size, text, "Ctrl+Alt+=");

            let tiles = crate::icons::size::shown(host.app());
            press(&mut host, Key::Command('='), ctrl());
            assert_eq!(crate::icons::size::shown(host.app()), tiles, "Ctrl+=");
            assert_eq!(
                shell::preferences(host.app()).text_size,
                text.stepped(1),
                "Ctrl+="
            );
        },
    );
}

/// The keys are read ahead of the rename editor, as GTK's `zoom_shortcut` is: the size changes
/// and the editor stays open with what was in it.
#[test]
fn the_text_size_keys_work_while_a_rename_is_open() {
    hosted_with(
        MODULE,
        "list",
        String::new(),
        "text_size::the_text_size_keys_work_while_a_rename_is_open",
        |mut host| {
            let folder = fixture();
            open(&mut host, super::LIST, folder.path());
            press(&mut host, Key::End, Mods::default());
            press(&mut host, Key::Function(2), Mods::default());
            let opened = host.run_until(PATIENCE, |host| field(host).is_some());
            assert!(opened, "the rename editor");
            let editing = field(&host);

            step(&mut host, '=', 14);
            assert_eq!(field(&host), editing, "the editor after Ctrl+=");
        },
    );
}
