// SPDX-License-Identifier: MIT

//! The reference on the window the crate registers: F1 opens it over the listing and gives
//! the keyboard back when it closes, and F1 in a rename is the rename's.

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, fixture, hosted, send_key},
};

const MODULE: &str = "shortcuts::tests";

fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
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

fn open(host: &Offscreen) -> bool {
    host.app().shortcuts.reference.is_open()
}

fn focused(host: &mut Offscreen) -> Option<String> {
    let mut focused = None;
    host.runtime().update(|_: &mut App, tx: &mut Tx| {
        focused = tx.focused().map(str::to_owned);
    });
    focused
}

/// Waits for the element keyed `key` to have the keyboard.
fn focused_on(host: &mut Offscreen, key: &str) -> bool {
    let deadline = std::time::Instant::now() + PATIENCE;
    while std::time::Instant::now() < deadline {
        if focused(host).as_deref() == Some(key) {
            return true;
        }
        host.turn();
    }
    false
}

#[test]
fn f1_opens_the_reference_and_escape_gives_the_keyboard_back() {
    hosted(
        MODULE,
        "f1_opens_the_reference_and_escape_gives_the_keyboard_back",
        |mut host| {
            let _folder = listed(&mut host);
            let view = shell::view_focus(host.app()).expect("a view").to_owned();
            assert_eq!(focused(&mut host).as_deref(), Some(view.as_str()));

            press(&mut host, Key::Function(1));
            assert!(open(&host), "F1 opens the reference");
            assert!(
                focused_on(&mut host, "shortcuts-search"),
                "the search has the keyboard"
            );
            let rename = host
                .app()
                .shortcuts
                .data
                .sections
                .iter()
                .flat_map(|section| &section.rows)
                .any(|row| row.action == "Rename" && row.key_text() == "F2 / Ctrl+R");
            assert!(rename, "the rows hold Rename with the keymap's keys");

            press(&mut host, Key::Escape);
            assert!(!open(&host), "Escape closes it");
            assert!(
                focused_on(&mut host, &view),
                "the listing has the keyboard back"
            );
        },
    );
}

#[test]
fn f1_in_a_rename_opens_nothing() {
    hosted(MODULE, "f1_in_a_rename_opens_nothing", |mut host| {
        let _folder = listed(&mut host);
        press(&mut host, Key::Home);
        press(&mut host, Key::Function(2));
        let editing = |host: &Offscreen| host.app().rename.state.is_open();
        assert!(host.run_until(PATIENCE, editing), "F2 opens the editor");

        press(&mut host, Key::Function(1));
        assert!(!open(&host), "F1 is the rename's");
        assert!(editing(&host), "the rename stays open");
    });
}
