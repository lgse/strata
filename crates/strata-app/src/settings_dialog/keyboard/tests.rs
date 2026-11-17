// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_desktop::keys::{
    Active, Binding, Command, Map,
    bindings::{
        ACTIONS_FOLDER, COPY, DUPLICATE, FILTER_SLASH, FIND, GO_HOME, RENAME, SEARCH_SUBFOLDERS,
        SORT_BY_NAME, YANK,
    },
    registry,
};

use super::{
    super::tests::{click, has_state, press},
    names::{name, title},
    rows::RowKeys,
    sections::sections,
};
use crate::{
    commands::{Origin, XKey},
    host::offscreen::Offscreen,
    shell,
    test_support::{
        hosted, send_key,
        view::{self, hosted_with, idle},
    },
};

const MODULE: &str = "settings_dialog::keyboard::tests";

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

fn type_text(host: &mut Offscreen, typed: &str) {
    for character in typed.chars() {
        send_key(
            host,
            KeyEvent {
                key: Key::Text(character.to_string()),
                mods: Mods::default(),
            },
        );
    }
    idle(host);
}

/// Opens Settings on the Keyboard page with `query` in the keybind search.
fn open_keyboard(host: &mut Offscreen, query: &str) {
    click(host, "header-settings");
    click(host, "settings-nav-keyboard");
    click(host, super::SEARCH_KEY);
    type_text(host, query);
}

fn focused(host: &Offscreen) -> Option<String> {
    let tree = host.tree();
    let nodes = view::all(&tree, &|node| has_state(node, "focused"));
    nodes
        .first()
        .and_then(|node| node["key"].as_str().map(str::to_owned))
}

fn named<'a>(tree: &'a Value, name: &str) -> Vec<&'a Value> {
    view::all(tree, &|node| view::text(node, "name") == name)
}

fn saved_rename(host: &Offscreen, map: Map) -> Option<toml::Value> {
    let keymap = &shell::preferences(host.app()).keymap;
    let entries = match map {
        Map::Default => &keymap.default,
        Map::Tenxer => &keymap.tenxer,
    };
    entries.get(RENAME.id()).cloned()
}

fn accelerators(list: &[&str]) -> toml::Value {
    toml::Value::Array(
        list.iter()
            .map(|a| toml::Value::String((*a).to_owned()))
            .collect(),
    )
}

fn live_binds(host: &Offscreen, binding: Binding) -> bool {
    shell::live_keys(host.app())
        .keymap()
        .bindings(Map::Default, RENAME)
        .contains(&binding)
}

fn parsed(accelerator: &str) -> Binding {
    Binding::parse(accelerator).expect("accelerator")
}

fn rename_keys() -> RowKeys {
    RowKeys::of(Map::Default, RENAME)
}

#[test]
fn a_taken_key_is_refused_with_its_owner_and_the_capture_keeps_waiting() {
    hosted(
        MODULE,
        "a_taken_key_is_refused_with_its_owner_and_the_capture_keeps_waiting",
        |mut host| {
            open_keyboard(&mut host, "Rename");
            click(&mut host, &rename_keys().add);
            assert_eq!(
                focused(&host).as_deref(),
                Some(rename_keys().capture.as_str())
            );

            press(&mut host, Key::Text("c".into()), ctrl());
            let tree = host.tree();
            let copy = title(COPY);
            let refusals = view::all(&tree, &|node| {
                node["role"] == "label"
                    && view::text(node, "name").contains("Ctrl+C")
                    && view::text(node, "name").contains(&copy)
            });
            assert_eq!(refusals.len(), 1, "one refusal naming Ctrl+C and Copy");
            let capture = named(&tree, "Press the new key for “Rename”");
            assert_eq!(capture.len(), 1);
            assert!(view::text(capture[0], "description").contains(&copy));
            assert_eq!(
                focused(&host).as_deref(),
                Some(rename_keys().capture.as_str())
            );
            assert_eq!(saved_rename(&host, Map::Default), None);

            press(&mut host, Key::Escape, Mods::default());
            assert!(
                host.app().settings_dialog.is_open(),
                "Escape ends only the capture"
            );
            assert_eq!(focused(&host).as_deref(), Some(rename_keys().add.as_str()));
            assert_eq!(saved_rename(&host, Map::Default), None);
        },
    );
}

#[test]
fn captured_and_removed_keys_save_and_dispatch_at_once() {
    hosted(
        MODULE,
        "captured_and_removed_keys_save_and_dispatch_at_once",
        |mut host| {
            open_keyboard(&mut host, "Rename");
            click(&mut host, &rename_keys().add);
            press(&mut host, Key::Text("e".into()), ctrl());
            assert_eq!(
                saved_rename(&host, Map::Default),
                Some(accelerators(&["F2", "<Control>r", "<Control>e"]))
            );
            assert!(live_binds(&host, parsed("<Control>e")));
            assert_eq!(focused(&host).as_deref(), Some(rename_keys().add.as_str()));

            let tree = host.tree();
            assert_eq!(named(&tree, "Remove Ctrl+E from “Rename”").len(), 1);
            click(
                &mut host,
                &format!("{}-chip-0", "settings-keymap-default-rename"),
            );
            assert_eq!(
                saved_rename(&host, Map::Default),
                Some(accelerators(&["<Control>r", "<Control>e"]))
            );
            assert!(!live_binds(&host, parsed("F2")));

            click(&mut host, &rename_keys().add);
            press(&mut host, Key::Backspace, Mods::default());
            assert_eq!(saved_rename(&host, Map::Default), Some(accelerators(&[])));
            assert!(!named(&host.tree(), "Not bound").is_empty());

            click(&mut host, "settings-keymap-default-rename-reset");
            assert_eq!(saved_rename(&host, Map::Default), None);
            assert!(live_binds(&host, parsed("F2")));
        },
    );
}

#[test]
fn a_russian_layout_records_the_latin_key_at_its_place() {
    hosted(
        MODULE,
        "a_russian_layout_records_the_latin_key_at_its_place",
        |mut host| {
            open_keyboard(&mut host, "Rename");
            click(&mut host, &rename_keys().add);
            // X keycode 26 is physical E, which types Cyrillic 'у' unshifted on `ru`.
            host.set_key_origin(Some(Origin::from_x(XKey {
                keycode: 26,
                base: Some('у'),
            })));
            press(&mut host, Key::Text("у".into()), ctrl());
            host.set_key_origin(None);
            assert_eq!(
                saved_rename(&host, Map::Default),
                Some(accelerators(&["F2", "<Control>r", "<Control>e"]))
            );
        },
    );
}

#[test]
fn each_table_edits_its_own_map_and_reset_all_asks_first() {
    hosted_with(
        MODULE,
        "columns",
        "[keymap.default]\nrename = [\"<Control>e\"]\n[keymap.tenxer]\nrename = [\"r\"]\n"
            .to_owned(),
        "each_table_edits_its_own_map_and_reset_all_asks_first",
        |mut host| {
            open_keyboard(&mut host, "Rename");
            click(&mut host, "settings-keymap-maps-1");
            assert!(!named(&host.tree(), "Remove r from “Rename”").is_empty());
            assert!(named(&host.tree(), "Remove Ctrl+E from “Rename”").is_empty());

            click(&mut host, super::RESET_ALL_KEY);
            assert!(!named(&host.tree(), "Reset all keybinds?").is_empty());
            press(&mut host, Key::Escape, Mods::default());
            assert!(host.app().settings_dialog.is_open());
            assert!(named(&host.tree(), "Reset all keybinds?").is_empty());
            assert_eq!(focused(&host).as_deref(), Some(super::RESET_ALL_KEY));
            assert_eq!(saved_rename(&host, Map::Tenxer), Some(accelerators(&["r"])));

            click(&mut host, super::RESET_ALL_KEY);
            click(&mut host, "settings-keymap-reset-all-confirm");
            assert_eq!(saved_rename(&host, Map::Tenxer), None);
            assert_eq!(
                saved_rename(&host, Map::Default),
                Some(accelerators(&["<Control>e"])),
                "the other table keeps its override"
            );
        },
    );
}

#[test]
fn the_search_moves_to_the_table_that_has_matches() {
    hosted(
        MODULE,
        "the_search_moves_to_the_table_that_has_matches",
        |mut host| {
            let find = title(FIND);
            open_keyboard(&mut host, &find);
            assert_eq!(host.app().settings_dialog.keyboard.shown, Map::Tenxer);
            let tree = host.tree();
            let tenxer = view::all(&tree, &|node| node["key"] == "settings-keymap-maps-1");
            assert!(has_state(tenxer[0], "checked"));

            type_text(&mut host, "qqqq");
            assert!(!named(&host.tree(), "No matching shortcuts").is_empty());
        },
    );
}

#[test]
fn tables_list_only_what_their_map_can_run() {
    let has = |map: Map, command: Command| {
        sections(map)
            .iter()
            .any(|(_, commands)| commands.contains(&command))
    };
    let default = sections(Map::Default);
    assert!(default.iter().all(|(title, _)| *title != "10xer mode"));
    assert!(!has(Map::Default, FIND), "a 10xer verb");
    assert!(!has(Map::Default, YANK), "a 10xer verb");
    assert!(!has(Map::Default, GO_HOME), "a chord step");
    assert!(
        !has(Map::Default, ACTIONS_FOLDER),
        "an Action step only the 10xer panel runs"
    );
    assert!(has(Map::Default, SORT_BY_NAME) && has(Map::Default, SEARCH_SUBFOLDERS));
    assert!(!has(Map::Tenxer, FILTER_SLASH), "a default-map key");
    assert!(
        has(Map::Tenxer, FIND) && has(Map::Tenxer, GO_HOME) && has(Map::Tenxer, ACTIONS_FOLDER)
    );
    assert!(has(Map::Tenxer, DUPLICATE));
}

#[test]
fn every_row_is_listed_once_and_has_its_own_name() {
    for map in [Map::Default, Map::Tenxer] {
        let listed: Vec<Command> = sections(map)
            .into_iter()
            .flat_map(|(_, commands)| commands)
            .collect();
        for command in registry() {
            let times = listed.iter().filter(|c| **c == command).count();
            assert!(times <= 1, "{map:?} lists {command:?} {times} times");
        }
        let mut names: Vec<String> = listed.iter().map(|c| title(*c)).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "{map:?} has two rows of one name");
    }
}

/// A command added to the catalog needs its row's name here, as GTK's `NAMES` gives it.
#[test]
fn every_command_has_a_row_name() {
    let unnamed: Vec<&str> = registry()
        .filter(|command| name(*command) == command.id())
        .map(Command::id)
        .collect();
    assert!(unnamed.is_empty(), "{unnamed:?}");
}
