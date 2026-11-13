// SPDX-License-Identifier: MIT

//! A click on a folder whose column is already open (finding lgse/strata#2356), as the
//! Columns fuzz pilot's shrunk sequence makes it: GTK's `activate_column_entry` reveals the
//! column, and its list, taking the keyboard with no cursor, focuses its first row.

use serde_json::json;
use strata_qa::Remembered;

use super::{launched, passes, staged};

#[test]
fn a_click_on_a_folder_already_open_puts_the_cursor_on_its_first_entry() {
    staged(
        "reopen::a_click_on_a_folder_already_open_puts_the_cursor_on_its_first_entry",
        None,
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            for step in [
                json!({ "key": "End" }),
                json!({ "key": "Up" }),
                json!({ "key": "Up" }),
                json!({ "wait": { "focused": "pictures" } }),
                json!({ "wait": { "entries": ["diagram.txt", "photo.txt"], "dir": "pictures" } }),
                json!({ "click": { "entry": "pictures" } }),
                json!({ "wait": { "focused": "diagram.txt" }, "timeout_ms": 3000 }),
                json!({ "expect": { "directory": "pictures" } }),
                json!({ "expect": { "selected": [], "dir": "pictures" } }),
                // A column with a cursor keeps it: Down to `photo.txt`, Left out, and the
                // click again.
                json!({ "key": "Down" }),
                json!({ "wait": { "focused": "photo.txt" } }),
                json!({ "key": "Left" }),
                json!({ "wait": { "focused": "pictures" } }),
                json!({ "click": { "entry": "pictures" } }),
                json!({ "wait": { "focused": "photo.txt" }, "timeout_ms": 3000 }),
                json!({ "expect": { "directory": "pictures" } }),
            ] {
                passes(&backend, &kept, &step);
            }
        },
    );
}
