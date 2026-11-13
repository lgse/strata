// SPDX-License-Identifier: MIT

//! Undo and redo through their keys, on the window the crate registers, in temporary
//! folders: what `inline_renaming`'s `rename-and-undo-restores` and `rename-moves-into-path`
//! press after their rename, with the rename asked of the browser directly.

use std::{collections::HashSet, path::Path};

use rune::{Key, KeyEvent, Mods};
use strata_core::model::Location;
use strata_fs::services::TypedPath;

use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, hosted, send_key},
};

const MODULE: &str = "undo::tests";

/// Opens the fixture and waits for the browser to list it.
fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    folder
}

fn act(host: &mut Offscreen, act: impl FnOnce(&mut App)) {
    host.runtime().update(|app, _| act(app));
    while host.turn() {}
}

/// Ctrl with `letter`, and Shift when `shift`, as Rune's window host sends a chord.
fn chord(host: &mut Offscreen, letter: char, shift: bool) {
    let mods = Mods {
        ctrl: true,
        cmd: true,
        shift,
        ..Mods::default()
    };
    send_key(
        host,
        KeyEvent {
            key: Key::Command(letter),
            mods,
        },
    );
    while host.turn() {}
}

/// Turns the host until `name` is in `folder` on disk, or is not.
fn wait_on_disk(host: &mut Offscreen, folder: &Path, name: &str, there: bool) {
    let path = folder.join(name);
    let reached = host.run_until(PATIENCE, |_| path.exists() == there);
    assert!(
        reached,
        "{name} still {}",
        if there { "absent" } else { "there" }
    );
}

/// The entry the browser lists under `name`.
fn entry(app: &App, name: &str) -> strata_core::model::FileEntry {
    let names = HashSet::from([name.to_owned()]);
    let mut entries = app.browser.entries_named(&names);
    entries.pop().expect("the entry")
}

/// `rename-and-undo-restores`: Ctrl+Z puts the old name back, and both redo keys of the
/// catalog rename it again.
#[test]
fn a_rename_is_undone_by_its_key_and_redone_by_either_of_redos() {
    hosted(
        MODULE,
        "a_rename_is_undone_by_its_key_and_redone_by_either_of_redos",
        |mut host| {
            let folder = listed(&mut host);
            act(&mut host, |app| {
                let todo = entry(app, "todo.txt");
                app.browser.rename(todo, "undo-target".to_owned());
            });
            wait_on_disk(&mut host, folder.path(), "undo-target", true);

            for (redo, shift) in [('z', true), ('y', false)] {
                chord(&mut host, 'z', false);
                wait_on_disk(&mut host, folder.path(), "todo.txt", true);
                assert!(!folder.path().join("undo-target").exists());

                chord(&mut host, redo, shift);
                wait_on_disk(&mut host, folder.path(), "undo-target", true);
                assert!(!folder.path().join("todo.txt").exists(), "Ctrl+{redo}");
            }
        },
    );
}

/// `rename-moves-into-path/columns-file`: a typed path moved the item into a folder it made.
/// Ctrl+Z checks the disk, moves the item back and takes the folder away.
#[test]
fn a_move_into_a_folder_it_made_is_undone_after_the_check() {
    hosted(
        MODULE,
        "a_move_into_a_folder_it_made_is_undone_after_the_check",
        |mut host| {
            let folder = listed(&mut host);
            let parent = Location::local(folder.path());
            act(&mut host, |app| {
                let todo = entry(app, "todo.txt");
                let path = TypedPath::parse("notes/todo.txt", false, false).expect("a path");
                app.browser.place_entry(todo, path.within(&parent), false);
            });
            wait_on_disk(&mut host, folder.path(), "notes/todo.txt", true);
            assert!(!folder.path().join("todo.txt").exists());

            chord(&mut host, 'z', false);

            wait_on_disk(&mut host, folder.path(), "todo.txt", true);
            wait_on_disk(&mut host, folder.path(), "notes", false);
        },
    );
}

/// With nothing to undo the key is not taken, and the next stage may read it.
#[test]
fn with_nothing_to_undo_the_key_is_not_taken() {
    hosted(
        MODULE,
        "with_nothing_to_undo_the_key_is_not_taken",
        |mut host| {
            let _folder = listed(&mut host);
            let mut outcome = None;
            host.runtime().update(|app, tx| {
                outcome = Some(run(app, "undo", On::Focus, tx));
            });
            assert_eq!(outcome, Some(Outcome::Declined));
        },
    );
}
