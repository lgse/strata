// SPDX-License-Identifier: MIT

//! A committed rename whose new name sorts elsewhere shows its row there
//! (`reveal_completed_rename`), in Columns and in a single pane: what
//! `rename_visibility/committed-rename-stays-visible` checks before its view switch.

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, folder_of, hosted_after, send_key},
};

const MODULE: &str = "rename::tests::reveal";

fn key(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn rows<'a>(node: &'a Value, found: &mut Vec<&'a Value>) {
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if child["role"] == "list-box-option" || child["role"] == "list-item" {
            found.push(child);
        }
        rows(child, found);
    }
}

/// Whether the row named `name` is drawn whole: mounted, and its visible part all of it.
fn shown_whole(host: &mut Offscreen, name: &str) -> bool {
    let tree = host.tree();
    let mut found = Vec::new();
    rows(&tree, &mut found);
    let Some(key) = found
        .iter()
        .find(|row| row["name"] == name)
        .and_then(|row| row["key"].as_str())
        .map(str::to_owned)
    else {
        return false;
    };
    host.runtime().elements().into_iter().any(|element| {
        !element.exiting
            && element.key.as_deref() == Some(key.as_str())
            && element.visible == element.rect
            && element.rect[3] > 0.0
    })
}

/// 80 entries, End to the last, F2, `a` over its name, Return: the row moves from the bottom
/// to the top, and is shown there.
fn renamed_to_the_top_is_shown(mode: &str) {
    let names: Vec<String> = (0..80).map(|i| format!("m-entry-{i:03}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let folder = folder_of(&names);
    let path = folder.path().to_owned();
    let test = format!("renamed_to_the_top_is_shown_in_{mode}");
    let saved = format!("mode = \"theme\"\ntheme = \"azure-glow\"\nbrowser_mode = \"{mode}\"\n");
    hosted_after(
        MODULE,
        &test,
        move || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            std::fs::write(file, saved).expect("settings");
        },
        move |mut host| {
            host.app().open(Location::local(&path));
            let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, counted), "the folder's listing");
            while host.turn() {}
            key(&mut host, Key::Down, Mods::default());
            key(&mut host, Key::End, Mods::default());
            let at_end = host.run_until(PATIENCE, |host| {
                host.app()
                    .browser()
                    .focused_entry()
                    .is_some_and(|entry| entry.display_name == "m-entry-079")
            });
            assert!(at_end, "End to the last entry");
            assert!(
                !shown_whole(&mut host, "m-entry-000"),
                "the view is at its bottom"
            );

            key(&mut host, Key::Function(2), Mods::default());
            assert!(
                host.run_until(PATIENCE, |host| host.app().rename.state.is_open()),
                "the editor"
            );
            let ctrl = Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            };
            key(&mut host, Key::Command('a'), ctrl);
            host.runtime().key(KeyEvent {
                key: Key::Text("a".to_owned()),
                mods: Mods::default(),
            });
            while host.turn() {}
            key(&mut host, Key::Enter, Mods::default());

            let renamed = path.join("a");
            assert!(host.run_until(PATIENCE, |_| renamed.exists()), "the rename");
            let mut whole = false;
            for _ in 0..200 {
                if shown_whole(&mut host, "a") {
                    whole = true;
                    break;
                }
                host.wait(std::time::Duration::from_millis(25));
                host.turn();
            }
            assert!(whole, "the renamed row is not shown whole in {mode}");
        },
    );
}

#[test]
fn renamed_to_the_top_is_shown_in_columns() {
    renamed_to_the_top_is_shown("columns");
}

#[test]
fn renamed_to_the_top_is_shown_in_list() {
    renamed_to_the_top_is_shown("list");
}
