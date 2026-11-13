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
