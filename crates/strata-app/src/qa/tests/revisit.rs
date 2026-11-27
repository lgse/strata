// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use strata_core::storage;
use strata_qa::Remembered;

use super::{Rune, launched, passes, staged_after};

/// How the folder was left before Return opens it again.
#[derive(Clone, Copy)]
enum Revisit {
    /// `pictures`, opened by a double click, left by Alt+Left: nothing was selected there.
    AfterADoubleClick,
    /// `documents`, where `spreadsheet.csv` was clicked, left by Alt+Up.
    AfterAClickInside,
}

impl Revisit {
    fn name(self) -> &'static str {
        match self {
            Self::AfterADoubleClick => {
                "a_folder_left_with_nothing_selected_comes_back_so_on_return"
            }
            Self::AfterAClickInside => {
                "a_folder_left_with_an_entry_selected_comes_back_so_on_return"
            }
        }
    }

    fn steps(self) -> Vec<Value> {
        match self {
            Self::AfterADoubleClick => vec![
                json!({ "click": { "entry": "pictures" }, "count": 2 }),
                json!({ "wait": { "directory": "pictures" } }),
                json!({ "key": "alt+Left" }),
                json!({ "wait": { "focused": "pictures" } }),
                json!({ "key": "Return" }),
                json!({ "wait": { "directory": "pictures" } }),
                json!({ "sleep": 300 }),
                json!({ "expect": { "focused": "diagram.txt" } }),
                json!({ "expect": { "selected": [] } }),
            ],
            Self::AfterAClickInside => vec![
                json!({ "click": { "entry": "documents" }, "count": 2 }),
                json!({ "wait": { "directory": "documents" } }),
                json!({ "click": { "entry": "spreadsheet.csv" } }),
                json!({ "wait": { "selected": ["spreadsheet.csv"] } }),
                json!({ "key": "alt+Up" }),
                json!({ "wait": { "focused": "documents" } }),
                json!({ "key": "Return" }),
                json!({ "wait": { "directory": "documents" } }),
                json!({ "sleep": 300 }),
                json!({ "expect": { "focused": "spreadsheet.csv" } }),
                json!({ "expect": { "selected": ["spreadsheet.csv"] } }),
            ],
        }
    }
}

/// Runs `revisit` in List on the fixture, then holds the window to F3's invariants.
fn comes_back_at_its_position(revisit: Revisit) {
    staged_after(
        &format!("revisit::{}", revisit.name()),
        None,
        || {
            let config = storage::config_directory();
            std::fs::create_dir_all(&config).expect("the config folder");
            std::fs::write(config.join("settings.toml"), "browser_mode = \"list\"\n")
                .expect("the settings");
        },
        |backend: Rune| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            for step in revisit.steps() {
                passes(&backend, &kept, &step);
            }
            let failed = passes(&backend, &kept, &json!({ "invariants": {} }));
            assert_eq!(failed, json!([]), "the invariants after the revisit");
        },
    );
}

#[test]
fn a_folder_left_with_nothing_selected_comes_back_so_on_return() {
    comes_back_at_its_position(Revisit::AfterADoubleClick);
}

#[test]
fn a_folder_left_with_an_entry_selected_comes_back_so_on_return() {
    comes_back_at_its_position(Revisit::AfterAClickInside);
}
