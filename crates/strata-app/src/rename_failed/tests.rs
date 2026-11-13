// SPDX-License-Identifier: MIT

//! A rename the browser fails, on the window the crate registers: GTK's dialog says why,
//! the window's commands wait under it, and closing it leaves the entry as it was.

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

/// GTK's `on_rename_failed`: a name the field accepts and the browser refuses, a taken one.
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
            let opened = host.run_until(PATIENCE, |host| field_text(host).is_some());
            assert!(opened, "the editor");
            key(&mut host, Key::Command('a'), ctrl());
            for character in "readme.md".chars() {
                host.runtime().key(KeyEvent {
                    key: Key::Text(character.to_string()),
                    mods: Mods::default(),
                });
            }
            while host.turn() {}
            key(&mut host, Key::Enter, Mods::default());

            let shown = host.run_until(PATIENCE, |host| dialog(host).is_some());
            let failed = dialog(&host);
            assert!(shown, "no dialog");
            let failed = failed.expect("the dialog");
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

            key(&mut host, Key::Enter, Mods::default());
            let closed = host.run_until(PATIENCE, |host| dialog(host).is_none());
            assert!(closed, "the dialog stayed: {:?}", dialog(&host));
            let read = |name: &str| std::fs::read_to_string(folder.path().join(name)).ok();
            assert_eq!(read("todo.txt").as_deref(), Some("todo.txt"));
            assert_eq!(read("readme.md").as_deref(), Some("readme.md"));
        },
    );
}
