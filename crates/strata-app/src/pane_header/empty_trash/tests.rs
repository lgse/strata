// SPDX-License-Identifier: MIT

//! The Trash's "Empty Trash" header button on the offscreen host: shown only over the Trash's
//! root, insensitive while it lists nothing, and asking to empty it when pressed. The Trash is
//! a folder of the test's own (`trash::empty::use_test_root`): a hosted test has no gvfs.

use std::path::PathBuf;

use rune::{Key, KeyEvent, Mods, Tx};
use serde_json::Value;

use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, fixture, send_key,
        view::{self, Shape, hosted_in, open},
    },
    trash::empty::use_test_root,
};

const MODULE: &str = "pane_header::empty_trash::tests";

const ICONS: Shape = Shape {
    described: "Icons view",
    list: "grid",
    entry: "grid-cell",
};

const LIST: Shape = Shape {
    described: "List view",
    list: "list",
    entry: "list-item",
};

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

fn greyed(node: &Value) -> bool {
    node["states"]
        .as_array()
        .is_some_and(|states| states.iter().any(|state| state == "disabled"))
}

/// A Trash of the test's own holding `items` files, with the Trash's root pointed at it.
fn trash(items: usize) -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").expect("the test's home"));
    let trash = home.join("own-trash");
    std::fs::create_dir_all(&trash).expect("the Trash");
    for n in 0..items {
        std::fs::write(trash.join(format!("item-{n}.txt")), "hello").expect("an item");
    }
    use_test_root(&trash);
    trash
}

/// The open dialog's name, if any.
fn dialog(host: &Offscreen) -> Option<String> {
    let tree = host.tree();
    let dialogs = view::all(&tree, &|node| node["role"] == "dialog");
    dialogs.first().map(|dialog| view::text(dialog, "name"))
}

/// Over a Trash that holds something the button is named and pressable, and Return on it asks
/// to empty the Trash.
#[test]
fn over_the_trash_it_asks_to_empty_it() {
    let name = "over_the_trash_it_asks_to_empty_it";
    hosted_in(MODULE, "icons", name, |mut host| {
        let trash = trash(2);
        open(&mut host, ICONS, &trash);
        let tree = host.tree();
        let button = node(&tree, "icons-empty-trash").expect("the Empty Trash button");
        assert_eq!(view::text(button, "role"), "button");
        assert_eq!(view::text(button, "name"), "Empty Trash");
        assert!(
            !greyed(button),
            "pressable over a Trash that holds something"
        );

        host.runtime()
            .update(|_: &mut App, tx: &mut Tx| tx.focus("icons-empty-trash", true));
        idle(&mut host);
        send_key(
            &mut host,
            KeyEvent {
                key: Key::Enter,
                mods: Mods::default(),
            },
        );
        let asked = host.run_until(PATIENCE, |host| {
            dialog(host).is_some_and(|name| name == "Empty Trash?")
        });
        assert!(asked, "Return on Empty Trash asks: {:?}", dialog(&host));
    });
}

/// Over an empty Trash, in the List view, the button shows and cannot be pressed.
#[test]
fn over_an_empty_trash_it_is_insensitive() {
    let name = "over_an_empty_trash_it_is_insensitive";
    hosted_in(MODULE, "list", name, |mut host| {
        let trash = trash(0);
        open(&mut host, LIST, &trash);
        let tree = host.tree();
        let button = node(&tree, "list-empty-trash").expect("the Empty Trash button");
        assert!(greyed(button), "insensitive over an empty Trash");
    });
}

/// Over any other folder there is no Empty Trash.
#[test]
fn over_another_folder_there_is_none() {
    let name = "over_another_folder_there_is_none";
    hosted_in(MODULE, "icons", name, |mut host| {
        let _trash = trash(1);
        let folder = fixture();
        open(&mut host, ICONS, folder.path());
        assert!(node(&host.tree(), "icons-up").is_some(), "the Icons header");
        assert!(node(&host.tree(), "icons-empty-trash").is_none());
    });
}
