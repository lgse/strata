// SPDX-License-Identifier: MIT

//! Move to Trash on the window the crate registers, in temporary folders: by Delete, and by
//! the entry menu's row on a folder whose column is open. What `delete-to-trash` and
//! `trashing-open-folder-opens-next` do.

use rune::{Key, KeyEvent, Mods};
use strata_core::model::Location;

use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, hosted, hosted_after, send_key},
};

const MODULE: &str = "trash::tests";

fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    folder
}

fn key(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

/// Selects the entry at `location` alone, as a click does, with the keyboard on it.
fn select(host: &mut Offscreen, location: &Location) {
    let location = location.clone();
    host.runtime().update(move |app: &mut App, tx| {
        let id = crate::shell::id_of(app, &location);
        let view = crate::shell::shown_view(app).expect("a view");
        (view.rows.select_only)(app, (0, id), tx);
    });
    while host.turn() {}
}

fn trashed_count() -> usize {
    let files = glib::user_data_dir().join("Trash/files");
    std::fs::read_dir(files).map_or(0, Iterator::count)
}

/// `delete-to-trash`: Delete moves the selected file to the Trash, with no dialog, and
/// Ctrl+Z brings it back. In a text field Delete is the field's.
#[test]
fn delete_moves_the_selected_file_to_the_trash() {
    hosted(
        MODULE,
        "delete_moves_the_selected_file_to_the_trash",
        |mut host| {
            let folder = listed(&mut host);
            let todo = folder.path().join("todo.txt");
            select(&mut host, &Location::local(&todo));

            key(&mut host, Key::Function(2), Mods::default());
            let editing = host.run_until(PATIENCE, |host| host.app().rename.state.is_open());
            assert!(editing, "the editor");
            key(&mut host, Key::Delete, Mods::default());
            assert!(todo.exists(), "Delete in the field trashes nothing");
            key(&mut host, Key::Escape, Mods::default());

            key(&mut host, Key::Delete, Mods::default());
            let gone = host.run_until(PATIENCE, |_| !todo.exists());
            assert!(gone, "todo.txt went nowhere");
            assert_eq!(trashed_count(), 1, "the Trash holds it");

            let ctrl = Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            };
            key(&mut host, Key::Command('z'), ctrl);
            assert!(host.run_until(PATIENCE, |_| todo.exists()), "undo");
        },
    );
}

/// `trashing-open-folder-opens-next`: the menu's row on a folder whose column is open moves
/// it to the Trash, and the keyboard goes on to the folder now in its place.
#[test]
fn trashing_an_open_folder_from_its_row_moves_on_to_the_next() {
    hosted(
        MODULE,
        "trashing_an_open_folder_from_its_row_moves_on_to_the_next",
        |mut host| {
            let folder = listed(&mut host);
            let archive = Location::local(folder.path().join("archive"));
            // A click on it: selected, and its column open.
            let target = archive.clone();
            host.runtime().update(move |app: &mut App, _| {
                let id = crate::shell::id_of(app, &target);
                let view = crate::shell::shown_view(app).expect("a view");
                let position = (view.rows.position)(app, (0, id)).expect("its row");
                app.browser.activate(0, position);
            });
            let opened = host.run_until(PATIENCE, |host| {
                host.app().browser().location_at(1).as_ref() == Some(&archive)
            });
            assert!(opened, "the folder's column");
            host.runtime().update(|app: &mut App, _| {
                app.browser.set_active_column(1);
            });
            while host.turn() {}

            let mut outcome = None;
            host.runtime().update(|app: &mut App, tx| {
                let row = On::Row((0, crate::shell::id_of(app, &archive)));
                outcome = Some(run(app, "trash", row, tx));
                // Before the deletion lands, the keyboard is on the folder's row above.
                assert_eq!(app.browser.active_depth(), Some(0));
                let focused = app.browser.focused_entry().map(|entry| entry.location);
                assert_eq!(focused.as_ref(), Some(&archive));
            });
            assert_eq!(outcome, Some(Outcome::Ran));
            let path = folder.path().join("archive");
            assert!(
                host.run_until(PATIENCE, |_| !path.exists()),
                "archive went nowhere"
            );
            let documents = Location::local(folder.path().join("documents"));
            let next = host.run_until(PATIENCE, |host| {
                host.app()
                    .browser()
                    .focused_entry()
                    .is_some_and(|entry| entry.location == documents)
            });
            assert!(next, "the cursor on the next folder");
        },
    );
}

/// `confirm_delete` outside Columns: Delete acts on the selection, or the folder the
/// keyboard entered, and never on the cursor's entry alone. After Escape clears the
/// selection in List, Delete trashes nothing.
#[test]
fn outside_columns_delete_with_nothing_selected_trashes_nothing() {
    hosted_after(
        MODULE,
        "outside_columns_delete_with_nothing_selected_trashes_nothing",
        || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = "mode = \"theme\"\ntheme = \"azure-glow\"\nbrowser_mode = \"list\"\n";
            std::fs::write(file, saved).expect("settings");
        },
        |mut host| {
            let folder = listed(&mut host);
            let todo = folder.path().join("todo.txt");
            select(&mut host, &Location::local(&todo));
            key(&mut host, Key::Escape, Mods::default());
            assert!(
                host.app().browser().selected_entries().is_empty(),
                "Escape cleared the selection"
            );

            key(&mut host, Key::Delete, Mods::default());
            host.run_until(std::time::Duration::from_millis(500), |_| false);
            assert!(todo.exists(), "Delete trashed the cursor's entry");
            assert_eq!(trashed_count(), 0);
        },
    );
}

fn nodes<'a>(node: &'a serde_json::Value, role: &str, found: &mut Vec<&'a serde_json::Value>) {
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if child["role"] == role {
            found.push(child);
        }
        nodes(child, role, found);
    }
}

/// The open dialog's name and the name of the button that has the keyboard.
fn dialog(host: &Offscreen) -> Option<(String, Option<String>)> {
    let tree = host.tree();
    let mut dialogs = Vec::new();
    nodes(&tree, "dialog", &mut dialogs);
    let dialog = dialogs.first()?;
    let mut buttons = Vec::new();
    nodes(dialog, "button", &mut buttons);
    nodes(dialog, "default-button", &mut buttons);
    let focused = buttons.iter().find(|button| {
        button["states"]
            .as_array()
            .is_some_and(|states| states.iter().any(|state| state == "focused"))
    });
    let name = |node: &serde_json::Value| node["name"].as_str().unwrap_or_default().to_owned();
    Some((name(dialog), focused.map(|button| name(button))))
}

fn wait_for_dialog(host: &mut Offscreen) -> (String, Option<String>) {
    let shown = host.run_until(PATIENCE, |host| dialog(host).is_some());
    let opened = dialog(host);
    assert!(shown, "no dialog");
    opened.expect("the dialog")
}

fn wait_for_no_dialog(host: &mut Offscreen) {
    let closed = host.run_until(PATIENCE, |host| dialog(host).is_none());
    assert!(closed, "the dialog stayed: {:?}", dialog(host));
}

/// `permanent-delete-symlinked-parent`'s keys: Shift+Delete asks, with the delete focused;
/// Escape keeps the file; asked again, Return deletes it for good, past the Trash.
#[test]
fn shift_delete_asks_and_deletes_for_good_only_when_confirmed() {
    hosted(
        MODULE,
        "shift_delete_asks_and_deletes_for_good_only_when_confirmed",
        |mut host| {
            let folder = listed(&mut host);
            let todo = folder.path().join("todo.txt");
            select(&mut host, &Location::local(&todo));
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };

            key(&mut host, Key::Delete, shift);
            let (name, focused) = wait_for_dialog(&mut host);
            assert_eq!(name, "Permanently delete 1 item?");
            assert_eq!(focused.as_deref(), Some("Permanently delete 1 item"));
            key(&mut host, Key::Escape, Mods::default());
            wait_for_no_dialog(&mut host);
            host.run_until(std::time::Duration::from_millis(300), |_| false);
            assert!(todo.exists(), "Escape deleted it");

            key(&mut host, Key::Delete, shift);
            wait_for_dialog(&mut host);
            key(&mut host, Key::Enter, Mods::default());
            assert!(host.run_until(PATIENCE, |_| !todo.exists()), "not deleted");
            wait_for_no_dialog(&mut host);
            assert_eq!(trashed_count(), 0, "it went past the Trash");
        },
    );
}

/// `report_partial_failure`: a trash the location could not do for some items, with nothing
/// else failed, asks to delete those for good, with Cancel focused since the user asked for
/// the Trash.
#[test]
fn a_trash_the_location_refused_asks_to_delete_for_good() {
    hosted(
        MODULE,
        "a_trash_the_location_refused_asks_to_delete_for_good",
        |mut host| {
            let folder = listed(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            host.runtime().update(|app: &mut App, _| {
                let entry = app
                    .browser
                    .entries_named(&std::collections::HashSet::from(["todo.txt".to_owned()]))
                    .pop()
                    .expect("the entry");
                app.trash.trashing = vec![entry];
                super::completed_with_errors(
                    app,
                    super::Failed {
                        retryable: vec![todo.clone()],
                        others_failed: false,
                    },
                );
            });
            let (name, focused) = wait_for_dialog(&mut host);
            assert_eq!(name, "Permanently delete 1 item?");
            assert_eq!(focused.as_deref(), Some("Cancel"));
            key(&mut host, Key::Enter, Mods::default());
            wait_for_no_dialog(&mut host);
            assert!(folder.path().join("todo.txt").exists(), "Cancel deleted it");
        },
    );
}
