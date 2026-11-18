// SPDX-License-Identifier: MIT

//! The open dialog is the one on top. Settings draws its messages inside itself, so while
//! "Reset all keybinds?" is open the snapshot names it, as GTK's names the message over its
//! Settings layer (the owner's ruling of 2026-11-17 on lgse/strata#2572).

use serde_json::json;
use strata_qa::Remembered;

use super::{harness_previews_off, launched, passes, staged_after};

#[test]
fn a_message_over_settings_is_the_open_dialog() {
    staged_after(
        "dialogs::a_message_over_settings_is_the_open_dialog",
        None,
        harness_previews_off,
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            let dialog = |backend: &_| {
                passes(backend, &kept, &json!({ "query": "state" }))["dialog"].clone()
            };
            for step in [
                json!({ "activate": { "role": "button", "name": "Settings" } }),
                json!({ "click": { "role": "button", "name": "Keyboard settings" } }),
                json!({ "activate": { "role": "button", "name": "Add a key for “Rename”", "rendered": false } }),
                json!({ "wait": { "focused_widget": "Press the new key for “Rename”" } }),
                json!({ "key": "ctrl+e" }),
                json!({ "activate": { "role": "button", "name": "Reset all", "rendered": false } }),
                json!({ "wait": { "present": { "role": "dialog", "name": "Reset all keybinds?" } } }),
            ] {
                passes(&backend, &kept, &step);
            }
            assert_eq!(dialog(&backend), json!("Reset all keybinds?"));

            let cancel = json!({ "activate": { "role": "button", "name": "Cancel", "within": { "role": "dialog", "name": "Reset all keybinds?" } } });
            passes(&backend, &kept, &cancel);
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "absent": { "role": "dialog", "name": "Reset all keybinds?" } } }),
            );
            assert_eq!(dialog(&backend), json!("Settings"));
        },
    );
}
