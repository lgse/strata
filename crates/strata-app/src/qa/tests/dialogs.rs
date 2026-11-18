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

/// `dialog_button` presses the message over Settings, the dialog on top: its Reset resets the
/// shown table and closes it, and Settings stays open beneath.
#[test]
fn a_dialog_button_presses_the_message_over_settings() {
    staged_after(
        "dialogs::a_dialog_button_presses_the_message_over_settings",
        None,
        harness_previews_off,
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            for step in [
                json!({ "activate": { "role": "button", "name": "Settings" } }),
                json!({ "click": { "role": "button", "name": "Keyboard settings" } }),
                json!({ "activate": { "role": "button", "name": "Add a key for “Rename”", "rendered": false } }),
                json!({ "wait": { "focused_widget": "Press the new key for “Rename”" } }),
                json!({ "key": "ctrl+e" }),
                json!({ "wait": { "presented": { "role": "button", "name": "Remove Ctrl+E from “Rename”", "rendered": false } } }),
                json!({ "activate": { "role": "button", "name": "Reset all", "rendered": false } }),
                json!({ "wait": { "present": { "role": "dialog", "name": "Reset all keybinds?" } } }),
                json!({ "click": { "dialog_button": "Reset" } }),
                json!({ "wait": { "absent": { "role": "dialog", "name": "Reset all keybinds?" } } }),
                json!({ "wait": { "not": { "presented": { "role": "button", "name": "Remove Ctrl+E from “Rename”", "rendered": false } } } }),
            ] {
                passes(&backend, &kept, &step);
            }
            let state = passes(&backend, &kept, &json!({ "query": "state" }));
            assert_eq!(state["dialog"], json!("Settings"));
        },
    );
}
