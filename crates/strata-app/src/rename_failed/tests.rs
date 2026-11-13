// SPDX-License-Identifier: MIT

//! A rename the browser fails, on the window the crate registers: GTK's dialog says why,
//! the window's commands wait under it, and closing it leaves the entry as it was.

use std::{collections::HashSet, path::Path};

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, hosted, send_key},
};

const MODULE: &str = "rename_failed::tests";

fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    folder
}

fn nodes<'a>(node: &'a Value, role: &str, found: &mut Vec<&'a Value>) {
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if child["role"] == role {
            found.push(child);
        }
        nodes(child, role, found);
    }
}

fn focused(node: &Value) -> bool {
    node["states"]
        .as_array()
        .is_some_and(|states| states.iter().any(|state| state == "focused"))
}

/// The open dialog: its name, the labels in it, and the name of what has the keyboard there.
#[derive(Debug)]
struct Dialog {
    name: String,
    labels: Vec<String>,
    focused: Option<String>,
}

fn dialog(host: &Offscreen) -> Option<Dialog> {
    let tree = host.tree();
    let mut dialogs = Vec::new();
    nodes(&tree, "dialog", &mut dialogs);
    let dialog = dialogs.first()?;
    let names = |role: &str| {
        let mut found = Vec::new();
        nodes(dialog, role, &mut found);
        found
    };
    let text = |node: &&Value| node["name"].as_str().unwrap_or_default().to_owned();
    let buttons = [names("button"), names("default-button")].concat();
    Some(Dialog {
        name: dialog["name"].as_str().unwrap_or_default().to_owned(),
        labels: names("label").iter().map(text).collect(),
        focused: buttons.iter().find(|node| focused(node)).map(text),
    })
}

fn field_text(host: &Offscreen) -> Option<String> {
    let tree = host.tree();
    let mut fields = Vec::new();
    nodes(&tree, "text-input", &mut fields);
    let field = fields.iter().find(|node| focused(node))?;
    Some(field["value"].as_str().unwrap_or_default().to_owned())
}

fn key(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

/// Types `name` over the whole of what the editor shows, and commits it.
fn retype(host: &mut Offscreen, name: &str) {
    let opened = host.run_until(PATIENCE, |host| field_text(host).is_some());
    assert!(opened, "the editor");
    key(host, Key::Command('a'), ctrl());
    for character in name.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
    }
    while host.turn() {}
    key(host, Key::Enter, Mods::default());
}

fn wait_for_dialog(host: &mut Offscreen) -> Dialog {
    let shown = host.run_until(PATIENCE, |host| dialog(host).is_some());
    let failed = dialog(host);
    assert!(shown, "no dialog");
    failed.expect("the dialog")
}

fn close_dialog(host: &mut Offscreen) {
    key(host, Key::Enter, Mods::default());
    let closed = host.run_until(PATIENCE, |host| dialog(host).is_none());
    assert!(closed, "the dialog stayed: {:?}", dialog(host));
}

fn names_on_disk(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .expect("the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("a name")
        })
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// The folder on disk, the browser's first column and the rows the view shows hold the same
/// names, hidden files left out.
fn assert_all_agree(host: &Offscreen, folder: &Path, after: &str) {
    let on_disk = names_on_disk(folder);
    let mut browser: Vec<String> = host
        .app()
        .browser()
        .with_column_entries(0, |entries| {
            entries
                .iter()
                .map(|entry| entry.display_name.clone())
                .filter(|name| !name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    browser.sort();
    assert_eq!(browser, on_disk, "the browser, after {after}");
    let tree = host.tree();
    let mut rows = Vec::new();
    nodes(&tree, "list-box-option", &mut rows);
    let shown: HashSet<String> = rows
        .iter()
        .map(|row| row["name"].as_str().unwrap_or_default().to_owned())
        .collect();
    let listed: HashSet<String> = on_disk.into_iter().collect();
    assert!(
        listed.is_subset(&shown),
        "the view, after {after}: {shown:?}"
    );
}

/// GTK's `on_rename_failed`: a name the field accepts and the browser refuses, a taken one.
/// Then the retry the dialog leaves room for: F2 again, and a free name goes through.
#[test]
fn a_failed_rename_shows_the_dialog_and_closing_it_leaves_the_entry_as_it_was() {
    hosted(
        MODULE,
        "a_failed_rename_shows_the_dialog_and_closing_it_leaves_the_entry_as_it_was",
        |mut host| {
            let folder = listed(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            host.runtime().update(|app: &mut App, tx| {
                let row = On::Row((0, crate::shell::id_of(app, &todo)));
                assert_eq!(run(app, "rename", row, tx), Outcome::Ran);
            });
            retype(&mut host, "readme.md");

            let failed = wait_for_dialog(&mut host);
            assert_eq!(failed.name, "Unable to rename item");
            // The browser's message, as GTK's dialog shows it.
            assert_eq!(failed.labels, ["“readme.md” already exists"]);
            assert_eq!(failed.focused.as_deref(), Some("Close"), "{failed:?}");
            assert_eq!(field_text(&host), None, "the editor stays closed");

            let hidden = host.app().browser().preferences().show_hidden;
            key(&mut host, Key::Command('h'), ctrl());
            assert_eq!(
                host.app().browser().preferences().show_hidden,
                hidden,
                "Ctrl+H waits under the dialog"
            );

            close_dialog(&mut host);
            let read = |name: &str| std::fs::read_to_string(folder.path().join(name)).ok();
            assert_eq!(read("todo.txt").as_deref(), Some("todo.txt"));
            assert_eq!(read("readme.md").as_deref(), Some("readme.md"));

            key(&mut host, Key::Function(2), Mods::default());
            retype(&mut host, "free.txt");
            let free = folder.path().join("free.txt");
            let renamed = host.run_until(PATIENCE, |host| {
                free.exists()
                    && host
                        .app()
                        .browser()
                        .focused_entry()
                        .is_some_and(|entry| entry.display_name == "free.txt")
            });
            assert!(renamed, "the retry, and the cursor on it");
            assert_eq!(read("free.txt").as_deref(), Some("todo.txt"));
            assert_eq!(read("todo.txt"), None);
            assert_all_agree(&host, folder.path(), "the failed rename and the retry");
        },
    );
}

/// GTK shows the dialog for every failed rename, whoever asked: here the undo of a rename
/// whose old name another file has taken since.
#[test]
fn a_failed_undo_of_a_rename_shows_the_dialog() {
    hosted(
        MODULE,
        "a_failed_undo_of_a_rename_shows_the_dialog",
        |mut host| {
            let folder = listed(&mut host);
            host.runtime().update(|app: &mut App, _| {
                let names = HashSet::from(["todo.txt".to_owned()]);
                let todo = app.browser.entries_named(&names).pop().expect("the entry");
                app.browser.rename(todo, "undo-target".to_owned());
            });
            let target = folder.path().join("undo-target");
            assert!(host.run_until(PATIENCE, |_| target.exists()), "the rename");
            std::fs::write(folder.path().join("todo.txt"), "in the way").expect("the file");

            key(&mut host, Key::Command('z'), ctrl());

            let failed = wait_for_dialog(&mut host);
            assert_eq!(failed.name, "Unable to rename item");
            assert!(target.exists(), "the undo moved nothing");
            assert_eq!(
                std::fs::read_to_string(folder.path().join("todo.txt"))
                    .ok()
                    .as_deref(),
                Some("in the way")
            );
        },
    );
}
