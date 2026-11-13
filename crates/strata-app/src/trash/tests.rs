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
            key(&mut host, Key::Delete, shift());
            let (name, focused) = wait_for_dialog(&mut host);
            assert_eq!(name, "Permanently delete 1 item?");
            assert_eq!(focused.as_deref(), Some("Permanently delete 1 item"));
            key(&mut host, Key::Escape, Mods::default());
            wait_for_no_dialog(&mut host);
            host.run_until(std::time::Duration::from_millis(300), |_| false);
            assert!(todo.exists(), "Escape deleted it");

            key(&mut host, Key::Delete, shift());
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

/// The key of the element that has the keyboard.
fn focused_key(host: &mut Offscreen) -> Option<String> {
    host.runtime()
        .elements()
        .into_iter()
        .find(|element| element.focused && !element.exiting)
        .and_then(|element| element.key)
}

fn view_key(host: &Offscreen) -> String {
    crate::shell::view_focus(host.app())
        .expect("a view")
        .to_owned()
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

/// `activate_focused_button`: Return presses the focused button only, so the delete is no
/// default button that Return would press with the keyboard elsewhere. (Rune's modal gives
/// the keyboard back to the dialog's first focus when it is dropped, so "nothing focused"
/// cannot be reached to press Return in; the role is what tells.)
#[test]
fn the_delete_is_no_default_button() {
    hosted(MODULE, "the_delete_is_no_default_button", |mut host| {
        let folder = listed(&mut host);
        select(&mut host, &Location::local(folder.path().join("todo.txt")));
        key(&mut host, Key::Delete, shift());
        wait_for_dialog(&mut host);

        let tree = host.tree();
        let mut defaults = Vec::new();
        nodes(&tree, "default-button", &mut defaults);
        let names: Vec<&str> = defaults
            .iter()
            .filter_map(|node| node["name"].as_str())
            .collect();
        assert_eq!(
            names,
            Vec::<&str>::new(),
            "a default button in the confirmation"
        );
    });
}

/// A confirmation asked while another is open waits for it, with its own first focus: a
/// partial failure's retry, which focuses Cancel, never takes the open dialog's place under
/// its focused delete. A confirm that races the event still deletes what it asked about.
#[test]
fn a_confirmation_asked_while_one_is_open_waits_for_it() {
    hosted(
        MODULE,
        "a_confirmation_asked_while_one_is_open_waits_for_it",
        |mut host| {
            let folder = listed(&mut host);
            let (todo, readme) = (
                folder.path().join("todo.txt"),
                folder.path().join("readme.md"),
            );
            select(&mut host, &Location::local(&todo));
            key(&mut host, Key::Delete, shift());
            wait_for_dialog(&mut host);

            let retry = Location::local(&readme);
            let refused = move |app: &mut App| {
                let names = std::collections::HashSet::from(["readme.md".to_owned()]);
                app.trash.trashing = app.browser.entries_named(&names);
                super::completed_with_errors(
                    app,
                    super::Failed {
                        retryable: vec![retry.clone()],
                        others_failed: false,
                    },
                );
            };
            let first = refused.clone();
            host.runtime().update(move |app: &mut App, _| first(app));
            while host.turn() {}
            let (name, focused) = dialog(&host).expect("the first dialog");
            assert_eq!(name, "Permanently delete 1 item?");
            assert_eq!(focused.as_deref(), Some("Permanently delete 1 item"));

            // The confirm, and the event racing it before the part's poll.
            host.runtime().update(move |app: &mut App, _| {
                super::confirmed(app);
                refused(app);
            });
            while host.turn() {}
            assert!(
                host.run_until(PATIENCE, |_| !todo.exists()),
                "the confirm was dropped"
            );
            let reopened = host.run_until(PATIENCE, |host| {
                dialog(host).is_some_and(|(_, focused)| focused.as_deref() == Some("Cancel"))
            });
            assert!(
                reopened,
                "the retry opened with Cancel: {:?}",
                dialog(&host)
            );
            key(&mut host, Key::Enter, Mods::default());
            wait_for_no_dialog(&mut host);
            assert!(readme.exists(), "Return deleted the retry set");
        },
    );
}

/// `remember_modal_focus_for_listing`: the keyboard goes back to the listing when the
/// confirmation closes, by Cancel and by the delete.
#[test]
fn closing_the_confirmation_gives_the_keyboard_back_to_the_listing() {
    hosted(
        MODULE,
        "closing_the_confirmation_gives_the_keyboard_back_to_the_listing",
        |mut host| {
            let folder = listed(&mut host);
            let todo = folder.path().join("todo.txt");
            select(&mut host, &Location::local(&todo));
            let view = view_key(&host);

            key(&mut host, Key::Delete, shift());
            wait_for_dialog(&mut host);
            key(&mut host, Key::Escape, Mods::default());
            wait_for_no_dialog(&mut host);
            while host.turn() {}
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(view.as_str()),
                "after Cancel"
            );

            key(&mut host, Key::Delete, shift());
            wait_for_dialog(&mut host);
            key(&mut host, Key::Enter, Mods::default());
            assert!(host.run_until(PATIENCE, |_| !todo.exists()), "the delete");
            wait_for_no_dialog(&mut host);
            while host.turn() {}
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(view.as_str()),
                "after the delete"
            );
        },
    );
}

/// Clicks the open menu's row named `name`, as a person does.
fn click_menu_row(host: &mut Offscreen, name: &str) {
    let tree = host.tree();
    let mut rows = Vec::new();
    nodes(&tree, "menu-item", &mut rows);
    let row = rows
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("no menu row {name}"));
    let key = row["key"].as_str().expect("a keyed row").to_owned();
    let elements = host.runtime().elements();
    let shown = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key.as_str()))
        .expect("the row is drawn");
    let [x, y, w, h] = shown.visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    while host.turn() {}
}

/// Opened from the entry menu, whose row is gone by the time the confirmation closes, the
/// keyboard still goes back to the listing (`remember_modal_focus_for_listing`).
#[test]
fn a_confirmation_from_the_menu_gives_the_keyboard_back_to_the_listing() {
    hosted(
        MODULE,
        "a_confirmation_from_the_menu_gives_the_keyboard_back_to_the_listing",
        |mut host| {
            let folder = listed(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            select(&mut host, &todo);
            let view = view_key(&host);
            host.runtime().update(|app: &mut App, tx| {
                let request = crate::menus::Request {
                    depth: 0,
                    entry: Some(crate::shell::id_of(app, &todo)),
                    anchor: strata_components::contract::Rect::new(60.0, 60.0, 0.0, 0.0),
                };
                crate::menus::open(app, request, tx);
            });
            while host.turn() {}
            click_menu_row(&mut host, "Permanently delete");
            wait_for_dialog(&mut host);

            key(&mut host, Key::Escape, Mods::default());
            wait_for_no_dialog(&mut host);
            while host.turn() {}
            assert_eq!(focused_key(&mut host).as_deref(), Some(view.as_str()));
        },
    );
}

/// A retry asked while a confirmation is open, for the same items that confirmation then
/// deletes, has nothing left to ask about and does not open.
#[test]
fn a_waiting_retry_for_what_was_just_deleted_does_not_open() {
    hosted(
        MODULE,
        "a_waiting_retry_for_what_was_just_deleted_does_not_open",
        |mut host| {
            let folder = listed(&mut host);
            let todo = folder.path().join("todo.txt");
            let location = Location::local(&todo);
            select(&mut host, &location);
            key(&mut host, Key::Delete, shift());
            wait_for_dialog(&mut host);

            host.runtime().update(move |app: &mut App, _| {
                let names = std::collections::HashSet::from(["todo.txt".to_owned()]);
                app.trash.trashing = app.browser.entries_named(&names);
                super::completed_with_errors(
                    app,
                    super::Failed {
                        retryable: vec![location],
                        others_failed: false,
                    },
                );
                super::confirmed(app);
            });
            while host.turn() {}
            assert!(host.run_until(PATIENCE, |_| !todo.exists()), "the delete");
            wait_for_no_dialog(&mut host);
            host.run_until(std::time::Duration::from_millis(300), |_| false);
            assert!(
                dialog(&host).is_none(),
                "a retry opened for what was deleted"
            );
        },
    );
}

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("HOME").expect("the test's home"))
}

/// A Trash of the test's own, holding `items` files and an empty folder, and the Trash root
/// pointed at it: a hosted test has no gvfs to list `trash:///` with. The folder is empty
/// because a plain folder refuses to delete a folder with something in it, where gvfs's Trash
/// deletes a whole trashed item.
fn seeded_trash(items: usize) -> std::path::PathBuf {
    let trash = home().join("seeded-trash");
    std::fs::create_dir_all(&trash).expect("the Trash");
    for n in 0..items {
        std::fs::write(trash.join(format!("item-{n}.txt")), "hello").expect("an item");
    }
    if items > 0 {
        std::fs::create_dir_all(trash.join("folder")).expect("a folder");
    }
    super::empty::use_test_root(&trash);
    trash
}

fn held(trash: &std::path::Path) -> usize {
    std::fs::read_dir(trash).map_or(0, Iterator::count)
}

fn empty_trash(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, tx| {
        assert_eq!(run(app, "empty-trash", On::Focus, tx), Outcome::Ran);
    });
    while host.turn() {}
}

/// `confirm_empty_trash`: the Trash is measured, then "Empty Trash?" asks with Empty Trash
/// focused; Escape keeps everything, and only the confirmation's Empty Trash empties it.
#[test]
fn empty_trash_asks_and_empties_only_when_confirmed() {
    hosted_after(
        MODULE,
        "empty_trash_asks_and_empties_only_when_confirmed",
        || {
            seeded_trash(2);
        },
        |mut host| {
            let trash = home().join("seeded-trash");
            assert_eq!(held(&trash), 3);
            empty_trash(&mut host);
            let (name, focused) = wait_for_dialog(&mut host);
            let confirming = host.run_until(PATIENCE, |host| {
                dialog(host).is_some_and(|(name, _)| name == "Empty Trash?")
            });
            assert!(confirming, "the confirmation, not {name:?}");
            let (_, focused_now) = dialog(&host).expect("the confirmation");
            assert_eq!(
                focused_now.as_deref(),
                Some("Empty Trash"),
                "first {focused:?}"
            );
            key(&mut host, Key::Escape, Mods::default());
            wait_for_no_dialog(&mut host);
            host.run_until(std::time::Duration::from_millis(300), |_| false);
            assert_eq!(held(&trash), 3, "Escape emptied the Trash");

            empty_trash(&mut host);
            let confirming = host.run_until(PATIENCE, |host| {
                dialog(host).is_some_and(|(name, _)| name == "Empty Trash?")
            });
            assert!(confirming, "the confirmation again");
            key(&mut host, Key::Enter, Mods::default());
            assert!(
                host.run_until(PATIENCE, |_| held(&trash) == 0),
                "the Trash still holds {}",
                held(&trash)
            );
            wait_for_no_dialog(&mut host);
            assert!(trash.exists(), "the Trash itself stays");
        },
    );
}

/// A press of Empty Trash that does not come from the confirmation on show empties nothing:
/// not while the Trash is still being measured, and not once the dialog has closed.
#[test]
fn only_the_open_confirmation_empties_the_trash() {
    hosted_after(
        MODULE,
        "only_the_open_confirmation_empties_the_trash",
        || {
            seeded_trash(1);
        },
        |mut host| {
            let trash = home().join("seeded-trash");
            host.runtime().update(|app: &mut App, tx| {
                run(app, "empty-trash", On::Focus, tx);
                super::empty::confirmed(app);
                assert_eq!(super::empty::shown(app), Some((true, "measuring")));
            });
            let confirming = host.run_until(PATIENCE, |host| {
                dialog(host).is_some_and(|(name, _)| name == "Empty Trash?")
            });
            assert!(confirming, "the confirmation");
            key(&mut host, Key::Escape, Mods::default());
            wait_for_no_dialog(&mut host);
            host.runtime()
                .update(|app: &mut App, _| super::empty::confirmed(app));
            host.run_until(std::time::Duration::from_millis(300), |_| false);
            assert_eq!(
                held(&trash),
                2,
                "a press off the open confirmation emptied the Trash"
            );
        },
    );
}

/// An empty Trash asks nothing: the measuring dialog closes when the walk finds nothing.
#[test]
fn an_empty_trash_asks_nothing() {
    hosted_after(
        MODULE,
        "an_empty_trash_asks_nothing",
        || {
            seeded_trash(0);
        },
        |mut host| {
            empty_trash(&mut host);
            let closed = host.run_until(PATIENCE, |host| {
                super::empty::shown(host.app()) == Some((false, "measuring"))
            });
            assert!(
                closed,
                "the measuring dialog closes: {:?}",
                super::empty::shown(host.app())
            );
        },
    );
}

/// A Trash that cannot be read says so (`finish_trash_summary`).
#[test]
fn a_trash_that_cannot_be_read_says_so() {
    hosted_after(
        MODULE,
        "a_trash_that_cannot_be_read_says_so",
        || super::empty::use_test_root(&home().join("no-such-trash")),
        |mut host| {
            empty_trash(&mut host);
            let said = host.run_until(PATIENCE, |host| {
                dialog(host).is_some_and(|(name, _)| name == "Unable to read Trash")
            });
            assert!(said, "the error, not {:?}", dialog(&host));
        },
    );
}
