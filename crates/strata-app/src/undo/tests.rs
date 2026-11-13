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

/// `rename-moves-into-path/columns-file` up to its undo: the typed path `notes/todo.txt` moves
/// the item into a folder it makes.
fn moved_into_notes(host: &mut Offscreen, folder: &Path) {
    let parent = Location::local(folder);
    act(host, |app| {
        let todo = entry(app, "todo.txt");
        let path = TypedPath::parse("notes/todo.txt", false, false).expect("a path");
        app.browser.place_entry(todo, path.within(&parent), false);
    });
    wait_on_disk(host, folder, "notes/todo.txt", true);
    assert!(!folder.join("todo.txt").exists());
}

/// What the undo command answers where the keyboard is, as Ctrl+Z asks it.
fn undo_now(host: &mut Offscreen) -> Option<Outcome> {
    let mut outcome = None;
    host.runtime().update(|app, tx| {
        outcome = Some(run(app, "undo", On::Focus, tx));
    });
    outcome
}

/// `rename-moves-into-path/columns-file`: Ctrl+Z checks the disk, moves the item back and
/// takes the folder the move made away.
#[test]
fn a_move_into_a_folder_it_made_is_undone_after_the_check() {
    hosted(
        MODULE,
        "a_move_into_a_folder_it_made_is_undone_after_the_check",
        |mut host| {
            let folder = listed(&mut host);
            moved_into_notes(&mut host, folder.path());

            chord(&mut host, 'z', false);

            wait_on_disk(&mut host, folder.path(), "todo.txt", true);
            wait_on_disk(&mut host, folder.path(), "notes", false);
        },
    );
}

/// An item now where the undo would put the moved one back: GTK asks, and with no prompt
/// here the replay stops as the prompt's Cancel does. The key is taken, nothing moves, the
/// item in the way keeps what it holds, and the entry is still there to undo.
#[test]
fn an_undo_with_an_item_in_the_way_moves_nothing_and_keeps_its_entry() {
    hosted(
        MODULE,
        "an_undo_with_an_item_in_the_way_moves_nothing_and_keeps_its_entry",
        |mut host| {
            let folder = listed(&mut host);
            moved_into_notes(&mut host, folder.path());
            let in_the_way = folder.path().join("todo.txt");
            std::fs::write(&in_the_way, "in the way").expect("the item in the way");

            assert_eq!(undo_now(&mut host), Some(Outcome::Ran));
            let answered = host.run_until(PATIENCE, |host| host.app().undo.checking.is_empty());
            assert!(answered, "the check never answered");
            while host.turn() {}

            assert_eq!(
                std::fs::read_to_string(folder.path().join("notes/todo.txt")).ok(),
                Some("todo.txt".to_owned()),
                "the moved item stays where the move put it"
            );
            assert_eq!(
                std::fs::read_to_string(&in_the_way).ok(),
                Some("in the way".to_owned())
            );
            assert!(host.app().browser().pending_undo_group().is_some());
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
            assert_eq!(undo_now(&mut host), Some(Outcome::Declined));
        },
    );
}
