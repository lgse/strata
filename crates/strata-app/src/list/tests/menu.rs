// SPDX-License-Identifier: MIT

//! The columns menu from the List header's Columns button (`append_columns_menu` in
//! `src/ui/browser_modes/list_pane.rs`).

use rune::{Key, KeyEvent, Mods, Tx};
use serde_json::Value;

use super::{LIST, MODULE, mixed, settle};
use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, send_key,
        view::{self, hosted_in, open},
    },
};

fn press(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    settle(host);
}

fn has(node: &Value, state: &str) -> bool {
    node["states"]
        .as_array()
        .is_some_and(|states| states.iter().any(|seen| seen == state))
}

/// The open menu's items: each one's name, description, and whether it is checked and
/// sensitive.
fn items(host: &Offscreen) -> Vec<(String, String, bool, bool)> {
    let tree = host.tree();
    let items = view::all(&tree, &|node| {
        node["role"]
            .as_str()
            .is_some_and(|role| role.starts_with("menu-item"))
    });
    items
        .into_iter()
        .map(|item| {
            (
                view::text(item, "name"),
                view::text(item, "description"),
                has(item, "checked"),
                !has(item, "disabled"),
            )
        })
        .collect()
}

fn saved_hidden() -> Option<toml::Value> {
    let path = strata_core::storage::config_directory().join("settings.toml");
    let text = std::fs::read_to_string(path).ok()?;
    text.parse::<toml::Table>()
        .ok()?
        .get("list_hidden_columns")
        .cloned()
}

/// Return on the header's Columns button opens the columns menu with the keyboard in it:
/// COLUMNS' check items, Name disabled with GTK's reason, and "Reset to default columns".
/// Return on Size hides the column, which is saved, and the menu stays open.
#[test]
fn return_on_the_columns_button_opens_the_columns_menu() {
    hosted_in(
        MODULE,
        "list",
        "menu::return_on_the_columns_button_opens_the_columns_menu",
        |mut host| {
            let folder = mixed();
            open(&mut host, LIST, folder.path());
            settle(&mut host);
            host.runtime().update(|_: &mut App, tx: &mut Tx| {
                tx.focus("list-columns-button", true);
            });
            settle(&mut host);
            press(&mut host, Key::Enter);

            let shown = |name: &str, description: &str, checked, sensitive| {
                (name.to_owned(), description.to_owned(), checked, sensitive)
            };
            assert_eq!(
                items(&host),
                [
                    shown("Name", "The Name column cannot be hidden", true, false),
                    shown("Mode", "", true, true),
                    shown("Size", "", true, true),
                    shown("Type", "", true, true),
                    shown("Modified", "", true, true),
                    shown("Reset to default columns", "", false, true),
                ]
            );

            press(&mut host, Key::Down);
            press(&mut host, Key::Enter);
            let saved = host.run_until(PATIENCE, |_| {
                saved_hidden().is_some_and(|hidden| hidden.to_string().contains("size"))
            });
            assert!(saved, "Size hidden and saved: {:?}", saved_hidden());
            let size = items(&host).into_iter().find(|item| item.0 == "Size");
            assert_eq!(
                size,
                Some(shown("Size", "", false, true)),
                "the menu stays open"
            );
        },
    );
}
