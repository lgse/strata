// SPDX-License-Identifier: MIT

//! A folder the user cannot read, opened from the listing, and a path that names nothing:
//! the window's error dialog says why, under GTK's two titles.

use std::os::unix::fs::PermissionsExt;

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, folder_of, hosted, send_key},
};

const MODULE: &str = "open_failed::tests";

/// The first dialog in the tree: its name, and the names of the labels in it.
fn dialog(node: &Value) -> Option<(String, Vec<String>)> {
    if node["role"] == "dialog" {
        let mut labels = Vec::new();
        collect_labels(node, &mut labels);
        return Some((node["name"].as_str().unwrap_or_default().to_owned(), labels));
    }
    let children = node["children"].as_array().map_or(&[][..], Vec::as_slice);
    children.iter().find_map(dialog)
}

fn collect_labels(node: &Value, labels: &mut Vec<String>) {
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if child["role"] == "label" {
            labels.push(child["name"].as_str().unwrap_or_default().to_owned());
        }
        collect_labels(child, labels);
    }
}

/// A folder holding `blocked`, which nobody may read, listed in the window.
fn with_blocked(host: &mut Offscreen) -> (tempfile::TempDir, Location) {
    let folder = folder_of(&["note.txt"]);
    let blocked = folder.path().join("blocked");
    std::fs::create_dir(&blocked).expect("the folder");
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    host.app().open(Location::local(folder.path()));
    let listed = |host: &Offscreen| {
        host.app().browser().with_column_entries(0, <[_]>::len) == Some(2)
    };
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    while host.turn() {}
    (folder, Location::local(blocked))
}

fn shown(host: &mut Offscreen) -> (String, Vec<String>) {
    let opened = host.run_until(PATIENCE, |host| dialog(&host.tree()).is_some());
    assert!(opened, "no dialog");
    dialog(&host.tree()).expect("the dialog")
}

fn closes_on_return(host: &mut Offscreen) {
    send_key(
        host,
        KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        },
    );
    let closed = host.run_until(PATIENCE, |host| dialog(&host.tree()).is_none());
    assert!(closed, "the dialog stayed");
}

fn unblock(blocked: &Location) {
    let path = blocked.native_path().expect("a path");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[test]
fn a_folder_refused_from_the_listing_and_a_path_refused_are_reported() {
    hosted(
        MODULE,
        "a_folder_refused_from_the_listing_and_a_path_refused_are_reported",
        |mut host| {
            let (folder, blocked) = with_blocked(&mut host);

            let locked = |host: &Offscreen| {
                let entries = host.app().browser().with_column_entries(0, |entries| {
                    entries.iter().position(|entry| entry.is_locked_folder())
                });
                entries.flatten()
            };
            assert!(host.run_until(PATIENCE, |host| locked(host).is_some()), "the fill");
            let position = locked(&host).expect("the locked folder");
            host.runtime().update(move |app: &mut App, _| app.browser.activate(0, position));
            let (title, labels) = shown(&mut host);
            assert_eq!(title, "Unable to open directory");
            assert!(
                labels.iter().any(|label| label.contains("do not have permission")),
                "the browser's reason: {labels:?}"
            );
            closes_on_return(&mut host);

            // A path that names nothing is refused once the browser has looked.
            let missing = folder.path().join("missing").display().to_string();
            host.runtime().update(move |app: &mut App, _| {
                let opened = app.browser.clone().navigate_input_located(&missing, None);
                assert!(opened.is_ok(), "refused before it was looked at");
            });
            let (title, _) = shown(&mut host);
            assert_eq!(title, "Unable to open location");
            closes_on_return(&mut host);
            unblock(&blocked);
        },
    );
}
