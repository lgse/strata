// SPDX-License-Identifier: MIT

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

fn holds_after_a_move_out(mode: &str, name: &str) {
    shown_in(mode, name, |backend, kept| {
        for step in [
            json!({ "select": { "entry": "todo.txt" } }),
            json!({ "drag": { "from": { "entry": "todo.txt" }, "to": { "entry": "archive" } } }),
            json!({ "wait": { "entry_gone": "todo.txt" } }),
            json!({ "wait": { "focused": "readme.md" } }),
            json!({ "expect": { "selected": [] } }),
        ] {
            passes(backend, kept, &step);
        }
        assert_eq!(failed(backend, kept), json!([]), "after the move out");
    });
}

#[test]
fn the_columns_hold_to_the_invariants_after_a_move_out() {
    holds_after_a_move_out(
        "columns",
        "the_columns_hold_to_the_invariants_after_a_move_out",
    );
}

#[test]
fn the_list_holds_to_the_invariants_after_a_move_out() {
    holds_after_a_move_out("list", "the_list_holds_to_the_invariants_after_a_move_out");
}

#[test]
fn an_unselected_cursor_stays_while_the_selection_moves_out() {
    shown_in(
        "list",
        "an_unselected_cursor_stays_while_the_selection_moves_out",
        |backend, kept| {
            for step in [
                json!({ "click": { "entry": "readme.md" } }),
                json!({ "click": { "entry": "todo.txt" }, "mods": ["ctrl"] }),
                json!({ "click": { "entry": "todo.txt" }, "mods": ["ctrl"] }),
                json!({ "wait": { "selected": ["readme.md"] } }),
                json!({ "wait": { "focused": "todo.txt" } }),
                json!({ "drag": { "from": { "entry": "readme.md" }, "to": { "entry": "archive" } } }),
                json!({ "wait": { "entry_gone": "readme.md" } }),
                json!({ "wait": { "focused": "todo.txt" } }),
                json!({ "expect": { "selected": [] } }),
            ] {
                passes(backend, kept, &step);
            }
            assert_eq!(failed(backend, kept), json!([]), "after the move out");
        },
    );
}

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

fn a_revisit_by_double_click_keeps_the_load_cursor<View: Revisited>() {
    shown_in(View::MODE, View::NAME, |backend, kept| {
        for step in [
            json!({ "wait": { "focused": "archive" } }),
            json!({ "key": View::NEXT }),
            json!({ "key": View::NEXT }),
            json!({ "wait": { "focused": "pictures" } }),
            json!({ "key": "Return" }),
            json!({ "wait": { "directory": "pictures" } }),
            json!({ "key": "alt+Left" }),
            json!({ "wait": { "focused": "pictures" } }),
            json!({ "click": { "entry": "pictures" }, "count": 2 }),
            json!({ "wait": { "directory": "pictures" } }),
            json!({ "sleep": 300 }),
        ] {
            passes(backend, kept, &step);
        }
        assert_eq!(failed(backend, kept), json!([]), "after the revisit");
    });
}

trait Revisited {
    const MODE: &'static str;
    const NEXT: &'static str;
    const NAME: &'static str;
}

struct InTheList;

impl Revisited for InTheList {
    const MODE: &'static str = "list";
    const NEXT: &'static str = "Down";
    const NAME: &'static str = "the_list_keeps_the_load_cursor_on_a_revisit_by_double_click";
}

struct InTheIcons;

impl Revisited for InTheIcons {
    const MODE: &'static str = "icons";
    const NEXT: &'static str = "Right";
    const NAME: &'static str = "the_icons_keep_the_load_cursor_on_a_revisit_by_double_click";
}

#[test]
fn the_list_keeps_the_load_cursor_on_a_revisit_by_double_click() {
    a_revisit_by_double_click_keeps_the_load_cursor::<InTheList>();
}

#[test]
fn the_icons_keep_the_load_cursor_on_a_revisit_by_double_click() {
    a_revisit_by_double_click_keeps_the_load_cursor::<InTheIcons>();
}

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
                    json!({ "click": { "name": "Appearance", "role": "toggle button" } }),
                    json!({ "wait": { "dialog": true } }),
                ),
                (
                    json!({ "key": "Escape" }),
                    json!({ "wait": { "not": { "dialog": true } } }),
                ),
                (
                    json!({ "click": { "header": "Choose sort field" } }),
                    json!({ "wait": { "menu_items": [] } }),
                ),
                (
                    json!({ "click": { "name": "Size", "role": "radio menu item" } }),
                    json!({ "wait": { "present": { "name": "Reset to default sort", "role": "menu item" } } }),
                ),
                (
                    json!({ "click": { "name": "Reset to default sort", "role": "menu item" } }),
                    json!({ "wait": { "not": { "present": { "name": "Reset to default sort", "role": "menu item" } } } }),
                ),
                (
                    json!({ "key": "Escape" }),
                    json!({ "wait": { "menu_items": null } }),
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

fn a_filter_s_results_are_held_to_what_the_tree_draws(mode: &str, name: &str) {
    shown_in(mode, name, |backend, kept| {
        for (step, then) in [
            (
                json!({ "click": { "entry": "readme.md" } }),
                json!({ "wait": { "focused": "readme.md" } }),
            ),
            (
                json!({ "key": "ctrl+f" }),
                json!({ "wait": { "field": {} } }),
            ),
            (json!({ "type": "no" }), json!({ "sleep": 500 })),
            (json!({ "key": "Down" }), json!({ "sleep": 300 })),
            (json!({ "key": "Escape" }), json!({ "sleep": 300 })),
            (json!({ "key": "Down" }), json!({ "sleep": 300 })),
        ] {
            passes(backend, kept, &step);
            passes(backend, kept, &then);
            assert_eq!(failed(backend, kept), json!([]), "after {step}");
        }
    });
}

#[test]
fn a_filter_s_results_in_the_columns_are_held_to_what_the_tree_draws() {
    a_filter_s_results_are_held_to_what_the_tree_draws(
        "columns",
        "a_filter_s_results_in_the_columns_are_held_to_what_the_tree_draws",
    );
}

#[test]
fn a_filter_s_results_in_the_list_are_held_to_what_the_tree_draws() {
    a_filter_s_results_are_held_to_what_the_tree_draws(
        "list",
        "a_filter_s_results_in_the_list_are_held_to_what_the_tree_draws",
    );
}

#[test]
fn a_filter_s_results_in_the_icons_are_held_to_what_the_tree_draws() {
    a_filter_s_results_are_held_to_what_the_tree_draws(
        "icons",
        "a_filter_s_results_in_the_icons_are_held_to_what_the_tree_draws",
    );
}

#[test]
fn a_hidden_tab_holds_what_its_browser_queues() {
    shown_in(
        "columns",
        "a_hidden_tab_holds_what_its_browser_queues",
        |backend, kept| {
            let hidden = backend.browser();
            let at = hidden.active_location();
            backend.update(|app, tx| {
                crate::shell::tabs::open(app, at, 1, tx);
            });
            for _ in 0..=crate::shell::tabs::HELD_EVENTS {
                hidden.toggle_hidden();
            }
            passes(backend, kept, &json!({ "sleep": 100 }));
            assert_eq!(failed(backend, kept), json!([]));
        },
    );
}

#[test]
fn a_filter_in_columns_keeps_the_load_cursor_and_a_hidden_view_clean() {
    shown_in(
        "columns",
        "a_filter_in_columns_keeps_the_load_cursor_and_a_hidden_view_clean",
        |backend, kept| {
            assert!(backend.browser().selection_is_load_cursor());
            for (step, then) in [
                (
                    json!({ "key": "ctrl+f" }),
                    json!({ "wait": { "field": {} } }),
                ),
                (json!({ "type": "no" }), json!({ "sleep": 500 })),
                (
                    json!({ "key": "ctrl+2" }),
                    json!({ "wait": { "view_mode": "Icons" } }),
                ),
                (
                    json!({ "key": "ctrl+1" }),
                    json!({ "wait": { "view_mode": "Columns" } }),
                ),
                (json!({ "key": "Escape" }), json!({ "sleep": 300 })),
            ] {
                passes(backend, kept, &step);
                passes(backend, kept, &then);
                assert_eq!(failed(backend, kept), json!([]), "after {step}");
            }
            assert!(backend.browser().selection_is_load_cursor());
            assert_eq!(backend.browser().location_at(1), None);
        },
    );
}

#[test]
fn a_chooser_s_footer_error_needs_no_keyboard_hold() {
    shown_in(
        "list",
        "a_chooser_s_footer_error_needs_no_keyboard_hold",
        |backend, _| {
            let mut dialogs_broken = Vec::new();
            backend.update(|app, _| {
                crate::chooser::mount_request(
                    app,
                    open_request(),
                    crate::chooser::source::ChooserFileSource::new(None),
                    (None, Box::new(|_| {})),
                );
                app.chooser.show_error("Choose a file");
                for dialog in ["Choose a file", "Delete"] {
                    let observation = strata_qa::Observation {
                        dialog: Some(dialog.to_owned()),
                        ..strata_qa::Observation::default()
                    };
                    let broken = crate::qa::invariants::check(app, &observation, true);
                    dialogs_broken.push(
                        broken
                            .iter()
                            .filter(|failure| failure.contains("a dialog"))
                            .count(),
                    );
                }
            });
            assert_eq!(dialogs_broken, [0, 1], "the footer error, then a dialog");
        },
    );
}

fn open_request() -> crate::portal::ChooserRequest {
    crate::portal::ChooserRequest {
        token: "fuzz".to_owned(),
        title: "Choose files".to_owned(),
        accept_label: "Open".to_owned(),
        modal: false,
        parent: None,
        parent_size_hint: None,
        initial_directory: std::env::temp_dir(),
        kind: crate::portal::ChooserKind::Open {
            directory: false,
            multiple: true,
        },
        filters: Vec::new(),
        current_filter: None,
        choices: Vec::new(),
    }
}
