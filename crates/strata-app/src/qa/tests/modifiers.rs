// SPDX-License-Identifier: MIT

//! The modifiers the shell holds as held down, as the offscreen host feeds them in the
//! steps' place of Rune's `on_modifiers` hook (packet entry 107). The windowed host's hook
//! needs a real window and cannot run headless.

use rune::Mods;
use serde_json::json;
use strata_qa::Remembered;

use super::{launched, passes, staged};
use crate::{shell, test_support::hosted};

const NONE: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};

const CTRL: Mods = Mods {
    cmd: true,
    ctrl: true,
    ..NONE
};

const CTRL_SHIFT: Mods = Mods {
    shift: true,
    ..CTRL
};

#[test]
fn the_shell_holds_the_modifiers_each_step_holds_and_lets_go() {
    staged(
        "modifiers::the_shell_holds_the_modifiers_each_step_holds_and_lets_go",
        None,
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            let mut heard = Vec::new();
            for step in [
                json!({ "key_down": "ctrl" }),
                json!({ "key_down": "shift" }),
                // A key struck while both are held leaves them held.
                json!({ "key": "Down" }),
                json!({ "key_up": "shift" }),
                json!({ "key_up": "ctrl" }),
                // A chord holds its modifiers only around its key.
                json!({ "key": "ctrl+shift+Down" }),
            ] {
                passes(&backend, &kept, &step);
                heard.push(backend.held_modifiers());
            }
            assert_eq!(heard, [CTRL, CTRL_SHIFT, CTRL_SHIFT, CTRL, NONE, NONE]);
        },
    );
}

#[test]
fn losing_window_focus_clears_held_modifiers() {
    hosted(
        "qa::tests::modifiers",
        "losing_window_focus_clears_held_modifiers",
        |mut host| {
            host.turn();
            host.modifiers(CTRL_SHIFT);
            assert_eq!(shell::held_modifiers(host.app()), CTRL_SHIFT);

            host.runtime().window_focus(false);
            host.turn();

            assert_eq!(shell::held_modifiers(host.app()), NONE);
        },
    );
}
