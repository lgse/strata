// SPDX-License-Identifier: MIT

//! Live click-activation preference updates and pointer-intent selection invariants through the
//! in-process QA backend.

use serde_json::json;
use strata_qa::{Backend as _, Remembered};

use super::{now, passes, staged_after};
use crate::{shell, test_support::fixture};

fn write_mode(mode: &str) {
    let file = strata_core::storage::config_directory().join("settings.toml");
    std::fs::create_dir_all(file.parent().expect("settings folder")).expect("settings folder");
    let saved = format!("browser_mode = {mode:?}\nsingle_click_previews = false\n");
    std::fs::write(file, saved).expect("settings");
}

#[test]
fn live_folder_click_preference_applies_without_restarting_in_list_view() {
    staged_after(
        "clicks::live_folder_click_preference_applies_without_restarting_in_list_view",
        None,
        || write_mode("list"),
        |backend| {
            let folder = fixture();
            backend.open(folder.path());
            now(backend.ready()).expect("the first listing");
            let kept = Remembered::default();

            passes(
                &backend,
                &kept,
                &json!({ "click": { "entry": "documents" } }),
            );
            passes(
                &backend,
                &kept,
                &json!({ "expect": { "selected": ["documents"] } }),
            );

            backend.update(|app, _| {
                shell::saved(app).change(|preferences| {
                    preferences.list_folder_clicks = 1;
                });
            });
            passes(
                &backend,
                &kept,
                &json!({ "click": { "entry": "pictures" } }),
            );
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "directory": "pictures" } }),
            );
        },
    );
}

#[test]
fn second_press_drag_and_ctrl_drag_keep_selection_without_activating_folder() {
    staged_after(
        "clicks::second_press_drag_and_ctrl_drag_keep_selection_without_activating_folder",
        None,
        || write_mode("icons"),
        |backend| {
            let folder = fixture();
            backend.open(folder.path());
            now(backend.ready()).expect("the first listing");
            let kept = Remembered::default();

            for step in [
                json!({ "click": { "entry": "documents" } }),
                json!({ "move": { "entry": "documents" } }),
                json!({ "button_down": 1 }),
                json!({ "move_by": [18, 0] }),
                json!({ "button_up": 1 }),
                json!({ "expect": { "selected": ["documents"] } }),
                json!({ "click": { "entry": "pictures" }, "mods": ["ctrl"] }),
                json!({ "expect": { "selected": ["documents", "pictures"] } }),
                json!({
                    "drag": {
                        "from": { "entry": "pictures" },
                        "to": { "entry": "archive" }
                    },
                    "mods": ["ctrl"]
                }),
                json!({ "expect": { "selected": ["documents", "pictures"] } }),
            ] {
                passes(&backend, &kept, &step);
            }
        },
    );
}
