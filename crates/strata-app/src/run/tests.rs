// SPDX-License-Identifier: MIT

//! "Run this program?" on the window the crate registers, in a temporary folder: what
//! `executable-requires-confirmation` and `executable-menu-confirmed-run` do, by the browser's
//! open and by the menu's row.

use std::{os::unix::fs::PermissionsExt, path::Path};

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    test_support::{PATIENCE, folder_of, hosted, send_key},
};

const MODULE: &str = "run::tests";

/// A program that leaves `ran` beside itself, in its own folder, when it runs.
fn program(folder: &Path) -> Location {
    let path = folder.join("run-me");
    std::fs::write(&path, "#!/bin/sh\nprintf ran > ran\n").expect("program");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
    Location::local(&path)
}

fn key(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    while host.turn() {}
}

fn opened(host: &mut Offscreen, location: &Location) {
    host.app().browser().open_location(location.clone());
    while host.turn() {}
}

fn asking(host: &Offscreen) -> bool {
    host.app().run.open
}

/// The tree holds a node named `name` that has the keyboard.
fn focused(node: &Value, name: &str) -> bool {
    let has_it = node["name"] == name
        && node["states"]
            .as_array()
            .is_some_and(|states| states.iter().any(|state| state == "focused"));
    has_it
        || node["children"]
            .as_array()
            .is_some_and(|children| children.iter().any(|child| focused(child, name)))
}

/// Waits for the confirmation to give its Run button the keyboard, as it opens.
fn run_focused(host: &mut Offscreen) {
    let shown = host.run_until(PATIENCE, |host| focused(&host.tree(), "Run"));
    assert!(shown, "Run has the keyboard when the confirmation opens");
}

#[test]
fn an_executable_runs_only_once_confirmed() {
    hosted(
        MODULE,
        "an_executable_runs_only_once_confirmed",
        |mut host| {
            let folder = folder_of(&["todo.txt"]);
            let run_me = program(folder.path());
            let ran = folder.path().join("ran");
            host.app().open(Location::local(folder.path()));
            let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, listed), "the folder's listing");
            while host.turn() {}

            opened(&mut host, &Location::local(folder.path().join("todo.txt")));
            assert!(!asking(&host), "a file that is not executable asks nothing");

            opened(&mut host, &run_me);
            assert!(asking(&host), "opening the program asks first");
            run_focused(&mut host);
            key(&mut host, Key::Escape);
            let closed = host.run_until(PATIENCE, |host| !asking(host));
            assert!(closed, "Escape closes the confirmation");

            let mut outcome = None;
            host.runtime().update(|app: &mut App, tx| {
                let row = On::Row((0, crate::shell::id_of(app, &run_me)));
                outcome = Some(run(app, "run", row, tx));
            });
            while host.turn() {}
            assert_eq!(outcome, Some(Outcome::Ran));
            assert!(asking(&host), "the Run row asks first");
            assert!(!ran.exists(), "nothing ran before Run");
            run_focused(&mut host);

            key(&mut host, Key::Enter);
            let closed = host.run_until(PATIENCE, |host| !asking(host));
            assert!(closed, "Run closes the confirmation");
            let started = host.run_until(PATIENCE, |_| ran.exists());
            assert!(started, "the program ran in its own folder");
        },
    );
}
