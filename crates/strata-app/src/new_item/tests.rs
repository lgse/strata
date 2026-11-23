// SPDX-License-Identifier: MIT

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

fn ctrl_alt() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        alt: true,
        ..Mods::default()
    }
}

fn ctrl_only() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

fn listed_in_root(host: &Offscreen, name: &str) -> bool {
    let browser = host.app().browser();
    let Some(snapshot) = browser.column_snapshot(0).filter(|s| !s.loading) else {
        return false;
    };
    browser
        .with_entries(0, 0..snapshot.count, |entries| {
            entries.iter().any(|entry| entry.display_name == name)
        })
        .unwrap_or(false)
}

fn select_at_zero(host: &mut Offscreen, names: &[&str]) {
    let owned: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    host.runtime().update(|app: &mut App, tx| {
        assert!(app.browser.select_entries_by_name_at(0, &owned));
        crate::shell::settle(app, tx);
    });
    while host.turn() {}
}

#[test]
fn ctrl_alt_n_falls_back_without_fill_and_groups_selection_into_one_undo_step() {
    hosted(
        MODULE,
        "ctrl_alt_n_falls_back_without_fill_and_groups_selection_into_one_undo_step",
        |mut host| {
            let folder = listed(&mut host);
            let root = folder.path();
            assert!(host.app().browser().selection_is_load_cursor());
            key(&mut host, Key::Command('n'), ctrl_alt());
            wait_for_field(&mut host, "new folder");
            assert!(root.join("readme.md").is_file());
            assert!(root.join("todo.txt").is_file());
            key(&mut host, Key::Escape, Mods::default());
            key(&mut host, Key::Command('z'), ctrl_only());
            assert!(host.run_until(PATIENCE, |h| {
                !root.join("new folder").exists()
                    && !h.app().browser().has_foreground_operation()
                    && !listed_in_root(h, "new folder")
            }));
            while host.turn() {}

            select_at_zero(&mut host, &["readme.md", "todo.txt"]);
            key(&mut host, Key::Command('n'), ctrl_alt());
            name_it(&mut host, "new folder", "grouped");

            wait_on_disk(&mut host, &root.join("grouped/readme.md"), true);
            wait_on_disk(&mut host, &root.join("grouped/todo.txt"), true);
            assert!(host.run_until(PATIENCE, |h| {
                !h.app().browser().has_foreground_operation() && listed_in_root(h, "grouped")
            }));
            while host.turn() {}
            assert!(!root.join("readme.md").exists());
            assert!(!root.join("todo.txt").exists());
            assert!(!root.join("new folder").exists());

            key(&mut host, Key::Command('z'), ctrl_only());
            wait_on_disk(&mut host, &root.join("readme.md"), true);
            wait_on_disk(&mut host, &root.join("todo.txt"), true);
            wait_on_disk(&mut host, &root.join("grouped"), false);
            assert!(!root.join("new folder").exists());
        },
    );
}

fn abandon_group_editor(host: &mut Offscreen, collide: bool) {
    wait_for_field(host, "new folder");
    if !collide {
        key(host, Key::Escape, Mods::default());
        return;
    }
    type_text(host, "archive");
    wait_for_field(host, "archive");
    key(host, Key::Enter, Mods::default());
    assert!(host.run_until(PATIENCE, |h| {
        crate::error_dialog::shown_details(h.app()).is_some()
    }));
    key(host, Key::Escape, Mods::default());
    assert!(crate::error_dialog::shown_details(host.app()).is_none());
}

fn check_abandoned_group_naming(test_name: &str, collide: bool) {
    hosted(MODULE, test_name, |mut host| {
        let folder = listed(&mut host);
        let root = folder.path();
        select_at_zero(&mut host, &["readme.md", "todo.txt"]);
        let readme = Location::local(root.join("readme.md"));
        let readme_id = crate::shell::id_of(host.app(), &readme);
        assert_eq!(
            run_now(&mut host, "group", On::Row((0, readme_id))),
            Some(Outcome::Ran)
        );
        abandon_group_editor(&mut host, collide);
        assert!(root.join("new folder/readme.md").is_file());
        assert!(root.join("new folder/todo.txt").is_file());

        select_at_zero(&mut host, &["new folder"]);
        key(&mut host, Key::Function(2), Mods::default());
        name_it(&mut host, "new folder", "later");
        assert!(host.run_until(PATIENCE, |h| {
            root.join("later/readme.md").is_file()
                && !h.app().browser().has_foreground_operation()
                && listed_in_root(h, "later")
        }));
        while host.turn() {}

        key(&mut host, Key::Command('z'), ctrl_only());
        assert!(host.run_until(PATIENCE, |h| {
            root.join("new folder/readme.md").is_file()
                && !h.app().browser().has_foreground_operation()
                && listed_in_root(h, "new folder")
        }));
        while host.turn() {}
        assert!(root.join("new folder/todo.txt").is_file());
        assert!(!root.join("later").exists());

        key(&mut host, Key::Command('z'), ctrl_only());
        wait_on_disk(&mut host, &root.join("readme.md"), true);
        wait_on_disk(&mut host, &root.join("todo.txt"), true);
        wait_on_disk(&mut host, &root.join("new folder"), false);
    });
}

#[test]
fn abandoned_group_naming_by_escape_leaves_later_rename_separate_in_undo() {
    check_abandoned_group_naming(
        "abandoned_group_naming_by_escape_leaves_later_rename_separate_in_undo",
        false,
    );
}

#[test]
fn failed_group_naming_by_collision_leaves_later_rename_separate_in_undo() {
    check_abandoned_group_naming(
        "failed_group_naming_by_collision_leaves_later_rename_separate_in_undo",
        true,
    );
}
