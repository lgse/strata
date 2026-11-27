// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, fixture, hosted, send_key},
};

const MODULE: &str = "window_keys::tests";

fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    folder
}

fn press(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    while host.turn() {}
}

fn focus(host: &mut Offscreen, key: impl FnOnce(&App) -> String) {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let key = key(app);
        tx.focus(key, true);
    });
    while host.turn() {}
}

fn selected(host: &Offscreen) -> Vec<String> {
    let entries = host.app().browser().selected_entries();
    entries
        .into_iter()
        .map(|entry| entry.display_name)
        .collect()
}

fn without_auto_preview() {
    let file = strata_core::storage::config_directory().join("settings.toml");
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
    let saved = "mode = \"theme\"\ntheme = \"azure-glow\"\nsingle_click_previews = false\n";
    std::fs::write(file, saved).expect("settings");
}

#[test]
fn escape_outside_the_files_clears_the_selection() {
    crate::test_support::hosted_after(
        MODULE,
        "escape_outside_the_files_clears_the_selection",
        without_auto_preview,
        |mut host| {
            let _folder = listed(&mut host);
            focus(&mut host, |app| {
                shell::view_focus(app).expect("a view").to_owned()
            });
            press(&mut host, Key::End);
            assert_eq!(selected(&host), ["todo.txt"]);

            focus(&mut host, |_| "header-appearance".to_owned());
            press(&mut host, Key::Escape);

            assert_eq!(selected(&host), Vec::<String>::new());
            let elements = host.runtime().elements();
            let focused = elements.into_iter().find(|e| e.focused).and_then(|e| e.key);
            assert_eq!(focused.as_deref(), Some("header-appearance"));
        },
    );
}

#[test]
fn escape_with_nothing_selected_closes_the_deepest_column_a_view_does_not_show() {
    hosted(
        MODULE,
        "escape_with_nothing_selected_closes_the_deepest_column_a_view_does_not_show",
        |mut host| {
            let _folder = listed(&mut host);
            focus(&mut host, |app| {
                shell::view_focus(app).expect("a view").to_owned()
            });
            press(&mut host, Key::Home);
            let opened = |host: &Offscreen| host.app().browser().location_at(1).is_some();
            assert!(host.run_until(PATIENCE, opened), "archive's column opens");

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                shell::show_view(app, "icons", tx);
            });
            while host.turn() {}
            focus(&mut host, |app| {
                shell::view_focus(app).expect("a view").to_owned()
            });
            press(&mut host, Key::Escape);
            assert_eq!(selected(&host), Vec::<String>::new());
            assert!(opened(&host), "the first Escape clears the selection only");

            press(&mut host, Key::Escape);
            assert!(!opened(&host), "the second Escape closes archive's column");
        },
    );
}

/// The keymap of `keymap/structural-aliases-act-as-enter-and-escape`, in List.
fn aliases_saved() {
    let file = strata_core::storage::config_directory().join("settings.toml");
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
    let saved = "mode = \"theme\"\ntheme = \"azure-glow\"\nbrowser_mode = \"list\"\n\
                 type_to_search = false\n[keymap.default]\nactivate = [\"Return\", \"o\"]\n\
                 escape = [\"Escape\", \"<Control>bracketleft\"]\n";
    std::fs::write(file, saved).expect("settings");
}

fn chord(host: &mut Offscreen, key: Key, ctrl: bool) {
    let mods = Mods {
        ctrl,
        cmd: ctrl,
        ..Mods::default()
    };
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

#[test]
fn structural_aliases_open_and_cancel_as_their_keys_do() {
    crate::test_support::hosted_after(
        MODULE,
        "structural_aliases_open_and_cancel_as_their_keys_do",
        aliases_saved,
        |mut host| {
            let folder = listed(&mut host);
            focus(&mut host, |app| {
                shell::view_focus(app).expect("a view").to_owned()
            });
            press(&mut host, Key::Home);
            press(&mut host, Key::Down);
            assert_eq!(selected(&host), ["documents"]);

            chord(&mut host, Key::Text("o".to_owned()), false);
            let documents = Location::local(folder.path().join("documents"));
            let opened = |host: &Offscreen| {
                let browser = host.app().browser();
                browser
                    .active_depth()
                    .and_then(|depth| browser.location_at(depth))
                    == Some(documents.clone())
            };
            assert!(host.run_until(PATIENCE, opened), "o opens documents");
            assert_eq!(
                host.app().browser().location_at(0),
                Some(documents.clone()),
                "in List, o opens in place, with no column of the root behind it"
            );

            let listed_documents = |host: &Offscreen| {
                let browser = host.app().browser();
                let depth = browser.active_depth().unwrap_or_default();
                browser
                    .column_entry_counts(depth)
                    .is_some_and(|counts| counts.total > 0)
            };
            assert!(
                host.run_until(PATIENCE, listed_documents),
                "documents lists"
            );
            press(&mut host, Key::Home);
            press(&mut host, Key::Function(2));
            let editing = |host: &Offscreen| host.app().rename.state.is_open();
            assert!(host.run_until(PATIENCE, editing), "F2 opens the editor");

            chord(&mut host, Key::Command('['), true);
            assert!(!editing(&host), "Ctrl+[ ends the rename");
            assert!(folder.path().join("documents/notes.txt").exists());
        },
    );
}
