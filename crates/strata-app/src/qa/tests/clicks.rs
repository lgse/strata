// SPDX-License-Identifier: MIT

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

/// Runs `steps` on the fixture with `mode` saved as the view to show.
fn runs(mode: &str, name: &str, steps: &[serde_json::Value]) {
    staged_after(
        &format!("clicks::{name}"),
        None,
        || write_mode(mode),
        |backend| {
            let folder = fixture();
            backend.open(folder.path());
            now(backend.ready()).expect("the first listing");
            let kept = Remembered::default();
            for step in steps {
                passes(&backend, &kept, step);
            }
        },
    );
}

fn shift_click_after_a_load_selects_the_entry_alone(mode: &str, name: &str) {
    runs(
        mode,
        name,
        &[
            json!({ "click": { "entry": "documents" }, "count": 2 }),
            json!({ "wait": { "directory": "documents" } }),
            json!({ "wait": { "focused": "notes.txt" } }),
            json!({ "click": { "entry": "report.md" }, "mods": ["shift"] }),
            json!({ "wait": { "selected": ["report.md"] }, "timeout_ms": 3000 }),
        ],
    );
}

fn shift_click_after_a_background_click_ranges_from_the_anchor(mode: &str, name: &str) {
    runs(
        mode,
        name,
        &[
            json!({ "click": { "entry": "readme.md" } }),
            json!({ "click": { "entry": "todo.txt" }, "mods": ["shift"] }),
            json!({ "wait": { "selected": ["readme.md", "todo.txt"] } }),
            json!({ "click": { "background": true } }),
            json!({ "wait": { "selected": [] } }),
            json!({ "click": { "entry": "todo.txt" }, "mods": ["shift"] }),
            json!({ "wait": { "selected": ["readme.md", "todo.txt"] }, "timeout_ms": 3000 }),
        ],
    );
}

#[test]
fn a_shift_click_after_a_load_in_the_table_selects_the_entry_alone() {
    shift_click_after_a_load_selects_the_entry_alone(
        "list",
        "a_shift_click_after_a_load_in_the_table_selects_the_entry_alone",
    );
}

#[test]
fn a_shift_click_after_a_load_in_the_grid_selects_the_entry_alone() {
    shift_click_after_a_load_selects_the_entry_alone(
        "icons",
        "a_shift_click_after_a_load_in_the_grid_selects_the_entry_alone",
    );
}

#[test]
fn a_shift_click_after_a_background_click_in_the_table_ranges_from_the_anchor() {
    shift_click_after_a_background_click_ranges_from_the_anchor(
        "list",
        "a_shift_click_after_a_background_click_in_the_table_ranges_from_the_anchor",
    );
}

#[test]
fn a_shift_click_after_a_background_click_in_the_grid_ranges_from_the_anchor() {
    shift_click_after_a_background_click_ranges_from_the_anchor(
        "icons",
        "a_shift_click_after_a_background_click_in_the_grid_ranges_from_the_anchor",
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

#[test]
fn a_shift_click_after_a_load_and_a_new_entry_selects_the_entry_alone() {
    staged_after(
        "clicks::a_shift_click_after_a_load_and_a_new_entry_selects_the_entry_alone",
        None,
        || write_mode("list"),
        |backend| {
            let folder = fixture();
            backend.open(folder.path());
            now(backend.ready()).expect("the first listing");
            let kept = Remembered::default();
            for step in [
                json!({ "click": { "entry": "documents" }, "count": 2 }),
                json!({ "wait": { "directory": "documents" } }),
                json!({ "wait": { "focused": "notes.txt" } }),
            ] {
                passes(&backend, &kept, &step);
            }
            let written = folder.path().join("documents").join("zz.txt");
            std::fs::write(written, "z\n").expect("a file behind the window");
            for step in [
                json!({ "wait": { "present": { "entry": "zz.txt" } } }),
                json!({ "click": { "entry": "report.md" }, "mods": ["shift"] }),
                json!({ "wait": { "selected": ["report.md"] }, "timeout_ms": 3000 }),
            ] {
                passes(&backend, &kept, &step);
            }
        },
    );
}

#[test]
fn a_shift_click_after_a_revisit_ranges_from_the_restored_cursor() {
    runs(
        "list",
        "a_shift_click_after_a_revisit_ranges_from_the_restored_cursor",
        &[
            json!({ "click": { "entry": "pictures" }, "count": 2 }),
            json!({ "wait": { "focused": "diagram.txt" } }),
            json!({ "key": "alt+Up" }),
            json!({ "wait": { "focused": "pictures" } }),
            json!({ "click": { "entry": "pictures" }, "count": 2 }),
            json!({ "wait": { "focused": "diagram.txt" } }),
            json!({ "click": { "entry": "photo.txt" }, "mods": ["shift"] }),
            json!({ "wait": { "selected": ["diagram.txt", "photo.txt"] }, "timeout_ms": 3000 }),
        ],
    );
}
