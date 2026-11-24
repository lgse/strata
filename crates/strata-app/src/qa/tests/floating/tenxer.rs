// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use strata_core::model::Location;
use strata_qa::{Backend as _, Remembered};

use super::super::{Rune, launched, passes, staged_after};
use crate::{
    commands::{self, On},
    shell,
};

/// A floating Copy to chooser on `todo.txt`, open with the keyboard on its first folder.
struct Chooser<'a> {
    backend: &'a Rune,
    kept: Remembered<Rune>,
    folder: tempfile::TempDir,
}

impl Chooser<'_> {
    fn step(&self, step: &Value) -> Value {
        passes(self.backend, &self.kept, step)
    }

    fn holds(&self, predicate: &Value) -> bool {
        self.step(&json!({ "holds": predicate })) == json!(true)
    }

    fn is_open(&self) -> bool {
        self.holds(&json!({ "present": { "toplevel": "Copy to" } }))
    }

    fn shows(&self, label: &str) -> bool {
        self.holds(&json!({ "present": { "role": "status bar", "name": label, "within": { "toplevel": "Copy to" } } }))
    }

    /// The role and name of what has the chooser's keyboard.
    fn keyboard(&self) -> Option<(&'static str, String)> {
        let main = self.backend.window()?;
        let floating = self
            .backend
            .toplevels()
            .into_iter()
            .find(|window| *window != main)?;
        let focus = self.backend.focus(&floating)?;
        Some((self.backend.role(&focus), self.backend.name(&focus)))
    }

    fn listing(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.folder.path())
            .expect("the fixture")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

/// Runs `run` in a floating chooser, launched with `settings` saved.
fn in_chooser(name: &str, settings: &'static str, run: impl FnOnce(&Chooser<'_>)) {
    staged_after(
        &format!("floating::tenxer::{name}"),
        None,
        || {
            let config = strata_core::storage::config_directory();
            std::fs::create_dir_all(&config).expect("the config folder");
            std::fs::write(config.join("settings.toml"), settings).expect("the settings");
        },
        |backend| {
            let folder = launched(&backend);
            let todo = Location::local(folder.path().join("todo.txt"));
            backend.update(move |app, tx| {
                let id = shell::id_of(app, &todo);
                commands::run(app, "copy-to", On::Row((0, id)), tx);
            });
            let chooser = Chooser {
                backend: &backend,
                kept: Remembered::default(),
                folder,
            };
            chooser.step(&json!({ "wait": { "present": { "role": "button", "name": "Copy here", "within": { "toplevel": "Copy to" } } } }));
            chooser.step(&json!({ "key": "Down" }));
            run(&chooser);
        },
    );
}

const TENXER: &str = "tenxer_mode = true\n";

fn space(name: &str, settings: &'static str, refused: bool) {
    in_chooser(name, settings, |chooser| {
        chooser.step(&json!({ "key": "space" }));
        assert_eq!(
            chooser.shows("Only one item can be chosen"),
            refused,
            "refused in 10xer mode only"
        );
    });
}

#[test]
fn space_in_the_floating_chooser_is_refused_in_tenxer_mode() {
    space(
        "space_in_the_floating_chooser_is_refused_in_tenxer_mode",
        TENXER,
        true,
    );
}

#[test]
fn space_in_the_floating_chooser_is_not_refused_in_default_mode() {
    space(
        "space_in_the_floating_chooser_is_not_refused_in_default_mode",
        "tenxer_mode = false\n",
        false,
    );
}

#[test]
fn the_window_s_commands_leave_a_tenxer_chooser_open() {
    in_chooser(
        "the_window_s_commands_leave_a_tenxer_chooser_open",
        TENXER,
        |chooser| {
            let before = chooser.listing();
            for keys in [
                &["ctrl+t"][..],
                &["t", "n"],
                &["ctrl+alt+n"],
                &["ctrl+z"],
                &["ctrl+comma"],
            ] {
                for key in keys {
                    chooser.step(&json!({ "key": key }));
                }
                chooser.step(&json!({ "sleep": 300 }));
                assert!(chooser.is_open(), "{keys:?} leaves the chooser open");
            }
            assert_eq!(chooser.listing(), before, "nothing made in the folder");
        },
    );
}

fn refused(name: &str, keys: &[&str]) {
    in_chooser(name, TENXER, |chooser| {
        for key in keys {
            chooser.step(&json!({ "key": key }));
        }
        assert!(
            chooser.shows("Not available in the file chooser"),
            "{keys:?} is refused"
        );
        assert!(chooser.is_open(), "{keys:?} leaves the chooser open");
    });
}

#[test]
fn the_actions_chord_is_refused_in_a_tenxer_chooser() {
    refused(
        "the_actions_chord_is_refused_in_a_tenxer_chooser",
        &["semicolon"],
    );
}

#[test]
fn yank_is_refused_in_a_tenxer_chooser() {
    refused("yank_is_refused_in_a_tenxer_chooser", &["y"]);
}

#[test]
fn close_window_is_refused_in_a_tenxer_chooser() {
    refused("close_window_is_refused_in_a_tenxer_chooser", &["shift+q"]);
}

#[test]
fn pinning_is_refused_in_a_tenxer_chooser() {
    refused("pinning_is_refused_in_a_tenxer_chooser", &["g", "plus"]);
}

#[test]
fn a_tenxer_chooser_s_own_keys_move_filter_and_cancel() {
    in_chooser(
        "a_tenxer_chooser_s_own_keys_move_filter_and_cancel",
        TENXER,
        |chooser| {
            let row = Some(("list item", "pictures".to_owned()));
            chooser.step(&json!({ "key": "Down" }));
            chooser.step(&json!({ "sleep": 300 }));
            assert_eq!(chooser.keyboard(), row, "Down reaches pictures");
            chooser.step(&json!({ "key": "l" }));
            chooser.step(&json!({ "sleep": 300 }));
            assert_ne!(chooser.keyboard(), row, "l enters pictures");
            chooser.step(&json!({ "key": "h" }));
            chooser.step(&json!({ "sleep": 300 }));
            assert_eq!(chooser.keyboard(), row, "h comes back to its row");
            let find = json!({ "role": "text", "name": "Find in this listing", "within": { "toplevel": "Copy to" } });
            for step in [
                json!({ "key": "slash" }),
                json!({ "wait": { "present": find } }),
                json!({ "key": "Escape" }),
                json!({ "wait": { "absent": find } }),
            ] {
                chooser.step(&step);
            }
            assert!(chooser.is_open(), "the first Escape closes only the filter");
            chooser.step(&json!({ "key": "Escape" }));
            chooser.step(&json!({ "wait": { "absent": { "toplevel": "Copy to" } } }));
            assert!(
                !chooser.folder.path().join("pictures/todo.txt").exists(),
                "Escape cancels"
            );
        },
    );
}
