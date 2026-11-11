// SPDX-License-Identifier: MIT

//! Gate G3's steps through the engine: the `field` target and predicate, the keys of the
//! rename editor, and the file on disk after them. Each test names the scenarios under
//! `tests/qa/scenarios/inline_renaming` whose steps it takes; an `fs` step is the runner's,
//! so the folder is read here instead.

use std::path::Path;

use serde_json::{Value, json};
use strata_qa::{Backend as _, Remembered};

use super::{launched, now, passes, staged};
use crate::{qa::backend::Rune, test_support::fixture};

fn all_pass(backend: &Rune, kept: &Remembered<Rune>, steps: &[Value]) {
    for step in steps {
        passes(backend, kept, step);
    }
}

/// The steps every rename scenario opens with: the editor on `name`, remembered as `rename`.
fn editor_on(name: &str) -> Vec<Value> {
    vec![
        json!({ "select": { "entry": "readme.md" } }),
        json!({ "select_with_keyboard": name }),
        json!({ "key": "F2" }),
        json!({ "wait": { "field": { "name": "Rename", "focused": true } } }),
        json!({ "remember": { "field": true }, "as": "rename" }),
    ]
}

fn closed() -> Value {
    json!({ "wait": { "absent": { "role": "text", "name": "Rename" } } })
}

fn shows(text: &str) -> Value {
    json!({ "wait": { "field": { "of": "rename", "text": text } } })
}

fn read(folder: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(folder.join(name)).ok()
}

/// `escape-preserves-name`, then `rename-commits-on-enter`.
#[test]
fn escape_leaves_the_name_and_return_renames_the_file() {
    staged(
        "rename::escape_leaves_the_name_and_return_renames_the_file",
        None,
        |backend| {
            let folder = launched(&backend);
            let kept = Remembered::default();

            all_pass(&backend, &kept, &editor_on("todo.txt"));
            all_pass(
                &backend,
                &kept,
                &[
                    json!({ "expect": { "field": { "of": "rename", "selection": [0, 4], "caret": 4 } } }),
                    json!({ "key": "ctrl+a" }),
                    shows("todo.txt"),
                    json!({ "type": "discarded" }),
                    shows("discarded"),
                    json!({ "key": "Escape" }),
                    closed(),
                    json!({ "wait": { "entry": "todo.txt" } }),
                    json!({ "wait": { "focused": "todo.txt" } }),
                ],
            );
            assert_eq!(read(folder.path(), "todo.txt").as_deref(), Some("todo.txt"));
            assert_eq!(read(folder.path(), "discarded"), None);

            all_pass(&backend, &kept, &editor_on("todo.txt"));
            all_pass(
                &backend,
                &kept,
                &[
                    json!({ "key": "ctrl+a" }),
                    shows("todo.txt"),
                    json!({ "type": "renamed.item" }),
                    shows("renamed.item"),
                    json!({ "key": "Return" }),
                    closed(),
                    json!({ "wait": { "entry": "renamed.item" } }),
                    json!({ "wait": { "entry_gone": "todo.txt" } }),
                    json!({ "wait": { "focused": "renamed.item" } }),
                    json!({ "wait": { "selected": ["renamed.item"] } }),
                ],
            );
            assert_eq!(
                read(folder.path(), "renamed.item").as_deref(),
                Some("todo.txt")
            );
            assert_eq!(read(folder.path(), "todo.txt"), None);
        },
    );
}

/// The offsets `long-rename-keeps-caret` reads, over a name whose characters take two and
/// three bytes: a caret counted in bytes would pass every ASCII scenario.
#[test]
fn the_caret_and_the_selection_count_characters_in_a_name_that_is_not_ascii() {
    staged(
        "rename::the_caret_and_the_selection_count_characters_in_a_name_that_is_not_ascii",
        None,
        |backend| {
            let folder = fixture();
            std::fs::write(folder.path().join("zō日本.txt"), "keep").expect("a file");
            backend.open(folder.path());
            now(backend.ready()).expect("the first listing");
            let kept = Remembered::default();
            let field = |check: Value| json!({ "wait": { "field": check } });

            all_pass(&backend, &kept, &editor_on("zō日本.txt"));
            all_pass(
                &backend,
                &kept,
                &[
                    field(
                        json!({ "of": "rename", "text": "zō日本.txt", "selection": [0, 4], "caret": 4 }),
                    ),
                    json!({ "expect": { "present": { "field": "rename" } } }),
                    json!({ "key": "End" }),
                    field(json!({ "of": "rename", "caret": 8, "selection": null })),
                    json!({ "type": "é" }),
                    field(json!({ "of": "rename", "text": "zō日本.txté", "caret": 9 })),
                    json!({ "key": "Left" }),
                    field(json!({ "of": "rename", "caret": 8 })),
                    json!({ "key": "shift+Home" }),
                    field(json!({ "of": "rename", "caret": 0, "selection": [0, 8] })),
                    json!({ "expect": { "editable_fields": 1 } }),
                    json!({ "expect": { "editor_within_viewport": { "of": "rename" } } }),
                    json!({ "key": "Escape" }),
                    json!({ "wait": { "editable_fields": 0 } }),
                ],
            );
            assert_eq!(read(folder.path(), "zō日本.txt").as_deref(), Some("keep"));
        },
    );
}

/// `rename-moves-into-path/columns-file-as-folder` and `invalid-names-retain-original`, then
/// `rename-commits-on-click-away`.
#[test]
fn a_refused_name_is_explained_in_the_field_and_a_click_away_commits_a_good_one() {
    staged(
        "rename::a_refused_name_is_explained_in_the_field_and_a_click_away_commits_a_good_one",
        None,
        |backend| {
            let folder = launched(&backend);
            let kept = Remembered::default();
            let refusal = |text: &str| json!({ "expect": { "description": { "target": { "field": "rename" }, "text": text } } });

            all_pass(&backend, &kept, &editor_on("todo.txt"));
            all_pass(
                &backend,
                &kept,
                &[
                    json!({ "key": "ctrl+a" }),
                    json!({ "type": "todo/" }),
                    shows("todo/"),
                    refusal("An existing file can’t become a folder"),
                    json!({ "key": "ctrl+a" }),
                    json!({ "key": "BackSpace" }),
                    json!({ "type": "bad//name" }),
                    shows("bad//name"),
                    refusal("Enter a name"),
                    json!({ "expect": { "editable_fields": 1 } }),
                    json!({ "click": { "background": true } }),
                    closed(),
                    json!({ "wait": { "entry": "todo.txt" } }),
                ],
            );
            assert_eq!(read(folder.path(), "todo.txt").as_deref(), Some("todo.txt"));
            assert!(!folder.path().join("bad").exists());

            all_pass(&backend, &kept, &editor_on("todo.txt"));
            all_pass(
                &backend,
                &kept,
                &[
                    json!({ "key": "ctrl+a" }),
                    json!({ "type": "renamed.item" }),
                    shows("renamed.item"),
                    json!({ "click": { "background": true } }),
                    closed(),
                    json!({ "wait": { "entry": "renamed.item" } }),
                ],
            );
            assert_eq!(
                read(folder.path(), "renamed.item").as_deref(),
                Some("todo.txt")
            );
            assert_eq!(read(folder.path(), "todo.txt"), None);
        },
    );
}

/// The context menu's Rename row, which the GTK view takes to the same editor as F2
/// (`rename_context_entry` in `src/ui/browser/context_menu/focus.rs`).
#[test]
fn the_menus_rename_row_opens_the_editor_on_its_entry() {
    staged(
        "rename::the_menus_rename_row_opens_the_editor_on_its_entry",
        None,
        |backend| {
            let folder = launched(&backend);
            let kept = Remembered::default();

            all_pass(
                &backend,
                &kept,
                &[
                    json!({ "select": { "entry": "readme.md" } }),
                    json!({ "click": { "entry": "todo.txt" }, "button": 3 }),
                    json!({ "wait": { "menu_items": {} } }),
                    json!({ "click": { "menu_item": "Rename" } }),
                    json!({ "wait": { "menu_items": null } }),
                    json!({ "wait": { "field": {
                        "name": "Rename", "focused": true, "text": "todo.txt", "selection": [0, 4]
                    } } }),
                    json!({ "wait": { "selected": ["todo.txt"] } }),
                    json!({ "type": "from-the-menu" }),
                    json!({ "key": "Return" }),
                    closed(),
                    json!({ "wait": { "entry": "from-the-menu.txt" } }),
                    json!({ "wait": { "focused": "from-the-menu.txt" } }),
                ],
            );
            assert_eq!(
                read(folder.path(), "from-the-menu.txt").as_deref(),
                Some("todo.txt")
            );
        },
    );
}
