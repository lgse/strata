// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use strata_core::storage;
use strata_qa::Remembered;

use super::{launched, passes, staged_after};

/// Runs `steps` on the fixture with `mode` saved as the view to show.
fn runs(mode: &str, name: &str, steps: &[Value]) {
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
            for step in steps {
                passes(&backend, &kept, step);
            }
        },
    );
}

/// The anchor on `archive`, the first entry: a right-click on `documents` leaves it there.
fn keeps_the_anchor(mode: &str, name: &str) {
    runs(
        mode,
        name,
        &[
            json!({ "wait": { "focused": "archive" } }),
            json!({ "click": { "entry": "documents" }, "button": 3 }),
            json!({ "wait": { "menu_items": {} } }),
            json!({ "key": "Escape" }),
            json!({ "wait": { "menu_items": null } }),
            json!({ "click": { "entry": "documents" }, "mods": ["shift"] }),
            json!({ "wait": { "selected": ["archive", "documents"] }, "timeout_ms": 3000 }),
        ],
    );
}

#[test]
fn a_right_click_in_a_column_with_no_anchor_anchors_there() {
    runs(
        "columns",
        "a_right_click_in_a_column_with_no_anchor_anchors_there",
        &[
            json!({ "click": { "entry": "pictures" } }),
            json!({ "wait": { "entries": ["diagram.txt", "photo.txt"], "dir": "pictures" } }),
            json!({ "click": { "entry": "photo.txt", "dir": "pictures" }, "button": 3 }),
            json!({ "wait": { "menu_items": {} } }),
            json!({ "key": "Escape" }),
            json!({ "wait": { "menu_items": null } }),
            json!({ "click": { "entry": "diagram.txt", "dir": "pictures" }, "mods": ["shift"] }),
            json!({ "wait": { "selected": ["diagram.txt", "photo.txt"], "dir": "pictures" }, "timeout_ms": 3000 }),
        ],
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
