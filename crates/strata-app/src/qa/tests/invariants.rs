// SPDX-License-Identifier: MIT

//! The invariants the fuzz runner asks for after every step (`qa/invariants.rs`): they hold
//! through ordinary steps in every view, and a browser that holds what the view does not show
//! breaks them.

use serde_json::{Value, json};
use strata_core::storage;
use strata_qa::Remembered;

use super::{Rune, launched, passes, staged_after};

/// Runs `test` on the fixture, launched with `mode` saved as the view to show.
fn shown_in(mode: &str, name: &str, test: impl FnOnce(&Rune, &Remembered<Rune>)) {
    staged_after(
        &format!("invariants::{name}"),
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
            test(&backend, &Remembered::default());
        },
    );
}

fn failed(backend: &Rune, kept: &Remembered<Rune>) -> Value {
    passes(backend, kept, &json!({ "invariants": {} }))
}

/// A click, a Shift range, a refresh and a Ctrl+click, each followed by the invariants.
fn holds_through_steps(mode: &str, name: &str) {
    shown_in(mode, name, |backend, kept| {
        assert_eq!(failed(backend, kept), json!([]), "at launch");
        for (step, then) in [
            (
                json!({ "click": { "entry": "readme.md" } }),
                json!({ "wait": { "selected": ["readme.md"] } }),
            ),
            (
                json!({ "click": { "entry": "todo.txt" }, "mods": ["shift"] }),
                json!({ "wait": { "selected": ["readme.md", "todo.txt"] } }),
            ),
            (json!({ "key": "F5" }), json!({ "sleep": 300 })),
            (
                json!({ "click": { "entry": "documents" }, "mods": ["ctrl"] }),
                json!({ "wait": { "selected": ["documents", "readme.md", "todo.txt"] } }),
            ),
        ] {
            passes(backend, kept, &step);
            passes(backend, kept, &then);
            assert_eq!(failed(backend, kept), json!([]), "after {step}");
        }
    });
}

#[test]
fn the_columns_hold_to_the_invariants_through_steps() {
    holds_through_steps(
        "columns",
        "the_columns_hold_to_the_invariants_through_steps",
    );
}

#[test]
fn the_list_holds_to_the_invariants_through_steps() {
    holds_through_steps("list", "the_list_holds_to_the_invariants_through_steps");
}

#[test]
fn the_icons_hold_to_the_invariants_through_steps() {
    holds_through_steps("icons", "the_icons_hold_to_the_invariants_through_steps");
}

/// Every column keeps its own load cursor: once Return has opened the folder the root's
/// load cursor is on, the root's still is one, while the browser's active column is the
/// folder's (seed 0x03 of `columns v3`, step 1).
#[test]
fn a_column_left_with_its_load_cursor_keeps_it() {
    shown_in(
        "columns",
        "a_column_left_with_its_load_cursor_keeps_it",
        |backend, kept| {
            passes(backend, kept, &json!({ "wait": { "focused": "archive" } }));
            passes(backend, kept, &json!({ "key": "Return" }));
            passes(
                backend,
                kept,
                &json!({ "wait": { "directory": "archive" } }),
            );
            assert_eq!(failed(backend, kept), json!([]));
        },
    );
}

/// The browser reports a column's marks again while another is active, here for a file
/// written into the root after Return opened `archive`: the root keeps its load cursor in
/// the view as in the browser (finding lgse/strata#2386; seed 0x07 of `calibration v3`).
#[test]
fn a_column_keeps_its_load_cursor_when_its_folder_changes_while_another_is_active() {
    shown_in(
        "columns",
        "a_column_keeps_its_load_cursor_when_its_folder_changes_while_another_is_active",
        |backend, kept| {
            passes(backend, kept, &json!({ "wait": { "focused": "archive" } }));
            passes(backend, kept, &json!({ "key": "Return" }));
            passes(
                backend,
                kept,
                &json!({ "wait": { "directory": "archive" } }),
            );
            let root = backend.browser().location_at(0).expect("the root");
            let root = root.native_path().expect("a local folder").to_owned();
            std::fs::write(root.join("fuzz-a.txt"), "fuzz\n").expect("a file behind the window");
            passes(
                backend,
                kept,
                &json!({ "wait": { "present": { "entry": "fuzz-a.txt" } } }),
            );
            assert_eq!(failed(backend, kept), json!([]));
        },
    );
}

/// The browser given a Shift anchor the view does not hold, as a handover that lost one
/// leaves it (findings lgse/strata#2354 and lgse/strata#2355): after a reload the view takes
/// the browser's, and a Shift range starts from an entry the user did not anchor.
#[test]
fn an_anchor_the_view_does_not_hold_breaks_them() {
    shown_in(
        "columns",
        "an_anchor_the_view_does_not_hold_breaks_them",
        |backend, kept| {
            passes(backend, kept, &json!({ "click": { "entry": "readme.md" } }));
            passes(
                backend,
                kept,
                &json!({ "wait": { "focused": "readme.md" } }),
            );
            assert_eq!(failed(backend, kept), json!([]));

            let browser = backend.browser();
            let todo = browser
                .with_column_entries(0, |entries| {
                    entries
                        .iter()
                        .position(|entry| entry.display_name == "todo.txt")
                })
                .flatten()
                .expect("todo.txt is listed");
            browser.set_selection_anchor(0, todo);

            assert_eq!(
                failed(backend, kept),
                json!([
                    "browser-view: column 0's anchor is Some(\"readme.md\") in the view and Some(\"todo.txt\") in the browser"
                ])
            );
        },
    );
}

/// The keyboard's holds follow a menu, a rename editor and a dialog as they open and close,
/// and the view switched away from does nothing while it is hidden.
#[test]
fn the_holds_follow_what_opens_and_a_hidden_view_does_nothing() {
    shown_in(
        "columns",
        "the_holds_follow_what_opens_and_a_hidden_view_does_nothing",
        |backend, kept| {
            for (step, then) in [
                (
                    json!({ "click": { "entry": "readme.md" }, "button": 3 }),
                    json!({ "wait": { "menu_items": {} } }),
                ),
                (
                    json!({ "key": "Escape" }),
                    json!({ "wait": { "menu_items": null } }),
                ),
                (
                    json!({ "key": "F2" }),
                    json!({ "wait": { "field": { "focused": true, "name": "Rename" } } }),
                ),
                (
                    json!({ "key": "Escape" }),
                    json!({ "wait": { "focused": "readme.md" } }),
                ),
                (
                    json!({ "key": "shift+Delete" }),
                    json!({ "wait": { "dialog": true } }),
                ),
                (
                    json!({ "key": "Escape" }),
                    json!({ "wait": { "not": { "dialog": true } } }),
                ),
                (
                    json!({ "key": "ctrl+2" }),
                    json!({ "wait": { "view_mode": "Icons" } }),
                ),
                (
                    json!({ "key": "ctrl+1" }),
                    json!({ "wait": { "view_mode": "Columns" } }),
                ),
            ] {
                passes(backend, kept, &step);
                passes(backend, kept, &then);
                assert_eq!(failed(backend, kept), json!([]), "after {step}");
            }
        },
    );
}

/// A filter's results take the place of a column's rows, and the browser holds none of their
/// selection, as on GTK: the invariants leave the marks and the pane of that column alone
/// while the results show, and hold everything else.
#[test]
fn a_column_showing_filter_results_is_held_to_all_but_its_marks() {
    shown_in(
        "columns",
        "a_column_showing_filter_results_is_held_to_all_but_its_marks",
        |backend, kept| {
            for (step, then) in [
                (
                    json!({ "click": { "entry": "readme.md" } }),
                    json!({ "wait": { "focused": "readme.md" } }),
                ),
                (
                    json!({ "key": "ctrl+f" }),
                    json!({ "wait": { "field": {} } }),
                ),
                (json!({ "type": "re" }), json!({ "sleep": 500 })),
                (json!({ "key": "Down" }), json!({ "sleep": 300 })),
                (json!({ "key": "Escape" }), json!({ "sleep": 300 })),
            ] {
                passes(backend, kept, &step);
                passes(backend, kept, &then);
                assert_eq!(failed(backend, kept), json!([]), "after {step}");
            }
        },
    );
}
