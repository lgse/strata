// SPDX-License-Identifier: MIT

//! The floating destination chooser as a second toplevel (`qa::windows`): the steps find it
//! by its title, keys go to it while it is in front, and the browser has no focus then.

use serde_json::json;
use strata_core::model::Location;
use strata_qa::{Backend as _, Remembered};

use super::{launched, passes, staged};
use crate::{
    commands::{self, On},
    shell,
};

#[test]
fn the_floating_chooser_is_a_toplevel_in_front_of_the_browser() {
    staged(
        "floating::the_floating_chooser_is_a_toplevel_in_front_of_the_browser",
        None,
        |backend| {
            let folder = launched(&backend);
            let todo = Location::local(folder.path().join("todo.txt"));
            backend.update(move |app, tx| {
                let id = shell::id_of(app, &todo);
                commands::run(app, "copy-to", On::Row((0, id)), tx);
            });
            let kept = Remembered::default();
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "present": { "toplevel": "Copy to" } } }),
            );
            let main = backend.window().expect("the browser window");
            let toplevels = backend.toplevels();
            let floating = toplevels
                .iter()
                .find(|window| **window != main)
                .expect("a second toplevel");
            assert_eq!(backend.title(floating).as_deref(), Some("Copy to"));
            assert_eq!(
                backend.focus(&main),
                None,
                "the browser is behind the chooser"
            );
            assert_eq!(
                passes(&backend, &kept, &json!({ "invariants": {} })),
                json!([]),
                "the chooser in front has the keyboard"
            );
            assert_eq!(backend.origin(floating).map(|(x, _)| x), Ok(8000.0));
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "present": { "role": "button", "name": "Copy here", "within": { "toplevel": "Copy to" } } } }),
            );
            passes(&backend, &kept, &json!({ "key": "Escape" }));
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "absent": { "toplevel": "Copy to" } } }),
            );
            assert_eq!(backend.toplevels(), std::slice::from_ref(&main));
            assert!(
                backend.focus(&main).is_some(),
                "the browser has the keys again"
            );
            assert!(folder.path().join("todo.txt").exists());
            assert!(
                !folder.path().join("documents/todo.txt").exists(),
                "Escape cancels"
            );
        },
    );
}

/// A right-click selects `todo.txt` and leaves the Shift anchor on `documents`, clicked
/// before; once Move to has taken `todo.txt` into `archive`, the row the view keeps the
/// keyboard on keeps that anchor too, as the browser does, and the invariants hold.
#[test]
fn a_move_through_the_chooser_keeps_the_browser_s_shift_anchor() {
    staged(
        "floating::a_move_through_the_chooser_keeps_the_browser_s_shift_anchor",
        None,
        |backend| {
            let folder = launched(&backend);
            let kept = Remembered::default();
            let archive = folder.path().join("archive");
            let within = json!({ "toplevel": "Move to" });
            let root = folder
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .expect("the folder's name")
                .to_owned();
            for step in [
                json!({ "click": { "entry": "documents" } }),
                json!({ "click": { "entry": "todo.txt" }, "button": 3 }),
                json!({ "wait": { "present": { "menu_item": "Move to…" } } }),
                json!({ "click": { "menu_item": "Move to…" } }),
                json!({ "wait": { "present": { "toplevel": "Move to" } } }),
                json!({ "key": "ctrl+l" }),
                json!({ "wait": { "present": { "field": true, "within": within } } }),
                json!({ "key": "ctrl+a" }),
                json!({ "type": archive.display().to_string() }),
                json!({ "key": "Return" }),
                json!({ "wait": { "present": { "role": "label", "name": "archive", "within": within } } }),
                json!({ "click": { "role": "button", "name": "Move here", "within": within } }),
                json!({ "wait": { "absent": { "toplevel": "Move to" } } }),
                json!({ "wait": { "absent": { "entry": "todo.txt", "dir": root } } }),
                json!({ "wait": { "present": { "entry": "todo.txt", "dir": "archive" } } }),
            ] {
                passes(&backend, &kept, &step);
            }
            assert!(archive.join("todo.txt").exists(), "the move lands");
            assert_eq!(
                passes(&backend, &kept, &json!({ "invariants": {} })),
                json!([]),
                "the view keeps the browser's anchor"
            );
        },
    );
}
