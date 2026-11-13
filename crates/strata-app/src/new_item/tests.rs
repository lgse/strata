// SPDX-License-Identifier: MIT

//! New Folder and New File on the window the crate registers, in temporary folders: the
//! item is made, the editor opens on it, and the name typed there is the one it keeps. What
//! `undo-created-folder-trashes` and `new-item-named-with-path` do, by the key and the row.

use std::{path::Path, time::Duration};

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, hosted, send_key},
};

const MODULE: &str = "new_item::tests";

fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    folder
}

fn text_inputs(node: &Value, found: &mut Vec<Value>) {
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if child["role"] == "text-input" {
            found.push(child.clone());
        }
        text_inputs(child, found);
    }
}

/// The text of the one field that has the keyboard, if one does.
fn focused_field(host: &Offscreen) -> Option<String> {
    let mut found = Vec::new();
    text_inputs(&host.tree(), &mut found);
    let focused = |node: &&Value| {
        node["states"]
            .as_array()
            .is_some_and(|states| states.iter().any(|state| state == "focused"))
    };
    let field = found.iter().find(focused)?;
    Some(field["value"].as_str().unwrap_or_default().to_owned())
}

fn wait_for_field(host: &mut Offscreen, text: &str) {
    let shown = |host: &Offscreen| focused_field(host).as_deref() == Some(text);
    let reached = host.run_until(PATIENCE, shown);
    assert!(
        reached,
        "the editor never showed {text:?}: {:?}",
        focused_field(host)
    );
    while host.turn() {}
}

fn key(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn type_text(host: &mut Offscreen, text: &str) {
    for character in text.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
    }
    while host.turn() {}
}

fn wait_on_disk(host: &mut Offscreen, path: &Path, there: bool) {
    let reached = host.run_until(PATIENCE, |_| path.exists() == there);
    assert!(
        reached,
        "{} {}",
        path.display(),
        if there { "never came" } else { "stayed" }
    );
}

/// Types `name` over the free name the editor proposes, and commits it.
fn name_it(host: &mut Offscreen, proposed: &str, name: &str) {
    wait_for_field(host, proposed);
    type_text(host, name);
    wait_for_field(host, name);
    key(host, Key::Enter, Mods::default());
}

fn run_now(host: &mut Offscreen, id: &str, on: On) -> Option<Outcome> {
    let mut outcome = None;
    host.runtime()
        .update(|app: &mut App, tx| outcome = Some(run(app, id, on, tx)));
    outcome
}

/// `undo-created-folder-trashes/columns-typed-name`: Ctrl+Shift+N makes `new folder`, the
/// editor opens on it, and the typed name is what it is called.
#[test]
fn ctrl_shift_n_makes_a_folder_that_takes_the_typed_name() {
    hosted(
        MODULE,
        "ctrl_shift_n_makes_a_folder_that_takes_the_typed_name",
        |mut host| {
            let folder = listed(&mut host);
            let held = Mods {
                ctrl: true,
                cmd: true,
                shift: true,
                ..Mods::default()
            };
            key(&mut host, Key::Command('n'), held);

            name_it(&mut host, "new folder", "plans");

            let plans = folder.path().join("plans");
            wait_on_disk(&mut host, &plans, true);
            assert!(plans.is_dir());
            assert!(!folder.path().join("new folder").exists());
        },
    );
}

/// `new-item-named-with-path/columns-file-to-folder`: the folder menu's New File makes
/// `new file`, and a trailing slash names it as a folder, which only a new item may become.
#[test]
fn a_new_file_named_with_a_trailing_slash_becomes_a_folder() {
    hosted(
        MODULE,
        "a_new_file_named_with_a_trailing_slash_becomes_a_folder",
        |mut host| {
            let folder = listed(&mut host);
            assert_eq!(
                run_now(&mut host, "new-file", On::Column(0)),
                Some(Outcome::Ran)
            );

            name_it(&mut host, "new file", "docs/");

            let docs = folder.path().join("docs");
            wait_on_disk(&mut host, &docs, true);
            assert!(docs.is_dir());
            assert!(!folder.path().join("new file").exists());
        },
    );
}

/// `begin_rename` gives up a naming that waits: a rename of another entry asked before the
/// new folder's editor opens keeps its editor and what is typed in it, and the folder keeps
/// its free name.
#[test]
fn a_rename_begun_while_a_new_item_waits_gives_the_naming_up() {
    hosted(
        MODULE,
        "a_rename_begun_while_a_new_item_waits_gives_the_naming_up",
        |mut host| {
            let folder = listed(&mut host);
            // Listed above where the new folder lands, so its row stays where it is.
            let archive = Location::local(folder.path().join("archive"));
            host.runtime().update(|app: &mut App, tx| {
                assert_eq!(run(app, "new-folder", On::Focus, tx), Outcome::Ran);
                let row = On::Row((0, crate::shell::id_of(app, &archive)));
                assert_eq!(run(app, "rename", row, tx), Outcome::Ran);
            });
            while host.turn() {}
            wait_for_field(&mut host, "archive");
            key(&mut host, Key::End, Mods::default());
            type_text(&mut host, ".kept");

            let new_folder = folder.path().join("new folder");
            wait_on_disk(&mut host, &new_folder, true);
            // Long enough for a naming that still waited to open its editor.
            host.run_until(Duration::from_millis(500), |_| false);
            assert_eq!(focused_field(&host).as_deref(), Some("archive.kept"));
            assert!(host.app().new_item.pending.is_none());

            key(&mut host, Key::Enter, Mods::default());
            wait_on_disk(&mut host, &folder.path().join("archive.kept"), true);
            assert!(new_folder.is_dir());
        },
    );
}
