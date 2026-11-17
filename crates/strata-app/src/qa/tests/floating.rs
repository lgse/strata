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
