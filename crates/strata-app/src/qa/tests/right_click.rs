// SPDX-License-Identifier: MIT

//! A right-click selects the entry it opens the menu on without moving the Shift anchor, in
//! every view, as GTK's `focus_context_entry` does (finding lgse/strata#2355): the Columns
//! fuzz pilot's shrunk sequence, step for step.

use serde_json::json;
use strata_core::storage;
use strata_qa::Remembered;

use super::{launched, passes, staged_after};

fn keeps_the_anchor(mode: &str, name: &str) {
    staged_after(
        &format!("right_click::{name}"),
        None,
        || {
            let config = storage::config_directory();
            std::fs::create_dir_all(&config).expect("the config folder");
            std::fs::write(
                config.join("settings.toml"),
                format!("browser_mode = \"{mode}\"\n"),
            )
            .expect("the settings");
        },
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            for step in [
                json!({ "wait": { "focused": "archive" } }),
                json!({ "click": { "entry": "documents" }, "button": 3 }),
                json!({ "wait": { "menu_items": {} } }),
                json!({ "key": "Escape" }),
                json!({ "wait": { "menu_items": null } }),
                json!({ "click": { "entry": "documents" }, "mods": ["shift"] }),
                json!({ "wait": { "selected": ["archive", "documents"] }, "timeout_ms": 3000 }),
            ] {
                passes(&backend, &kept, &step);
            }
        },
    );
}

#[test]
fn a_right_click_in_the_columns_leaves_the_shift_anchor() {
    keeps_the_anchor(
        "columns",
        "a_right_click_in_the_columns_leaves_the_shift_anchor",
    );
}

#[test]
fn a_right_click_in_the_grid_leaves_the_shift_anchor() {
    keeps_the_anchor("icons", "a_right_click_in_the_grid_leaves_the_shift_anchor");
}

#[test]
fn a_right_click_in_the_table_leaves_the_shift_anchor() {
    keeps_the_anchor("list", "a_right_click_in_the_table_leaves_the_shift_anchor");
}
