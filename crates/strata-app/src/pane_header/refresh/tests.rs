// SPDX-License-Identifier: MIT

//! The headers' Refresh on the offscreen host, in each view: named for its key, and reading the
//! folder again when pressed, from the pane header by Return and from a Columns column's
//! header by a click.

use rune::{Key, KeyEvent, Mods, Tx};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, folder_of, send_key, view},
};

const MODULE: &str = "pane_header::refresh::tests";

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn node<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

fn press(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    idle(host);
}

fn click(host: &mut Offscreen, key: &str) {
    let elements = host.runtime().elements();
    let found = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = found.unwrap_or_else(|| panic!("{key} is drawn")).visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

/// Opens a folder of files in the view and waits for the header's Refresh, keyed `key`. Files
/// only: in Columns a selected folder opens beside, and its read would pass for a refresh.
fn opened(host: &mut Offscreen, key: &str) -> tempfile::TempDir {
    let folder = folder_of(&["a.txt", "b.txt", "c.txt"]);
    host.app().open(Location::local(folder.path()));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the listing");
    idle(host);
    // A Columns column shows its header actions once input targets it.
    press(host, Key::Down);
    let shown = host.run_until(PATIENCE, |host| node(&host.tree(), key).is_some());
    assert!(shown, "{key} in the header");
    let tree = host.tree();
    let button = node(&tree, key).expect("the button");
    assert_eq!(view::text(button, "name"), "Refresh (F5)");
    folder
}

/// Waits for the browser to read the folder again after `refresh`.
fn reads_again(host: &mut Offscreen, refresh: impl FnOnce(&mut Offscreen)) {
    while host.turn() {}
    let copied = host.app().rows_copied();
    refresh(host);
    let read = host.run_until(PATIENCE, |host| host.app().rows_copied() > copied);
    assert!(read, "the folder was read again");
}

fn return_on(host: &mut Offscreen, key: &'static str) {
    host.runtime()
        .update(move |_: &mut App, tx: &mut Tx| tx.focus(key, true));
    idle(host);
    press(host, Key::Enter);
}

#[test]
fn the_icons_headers_refresh_reads_the_folder_again() {
    let name = "the_icons_headers_refresh_reads_the_folder_again";
    view::hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = opened(&mut host, "icons-refresh");
        reads_again(&mut host, |host| return_on(host, "icons-refresh"));
    });
}

#[test]
fn the_list_headers_refresh_reads_the_folder_again() {
    let name = "the_list_headers_refresh_reads_the_folder_again";
    view::hosted_in(MODULE, "list", name, |mut host| {
        let _folder = opened(&mut host, "list-refresh");
        reads_again(&mut host, |host| return_on(host, "list-refresh"));
    });
}

#[test]
fn a_columns_headers_refresh_reads_the_folder_again() {
    let name = "a_columns_headers_refresh_reads_the_folder_again";
    view::hosted_in(MODULE, "columns", name, |mut host| {
        let _folder = opened(&mut host, "columns-refresh-0");
        reads_again(&mut host, |host| click(host, "columns-refresh-0"));
    });
}
