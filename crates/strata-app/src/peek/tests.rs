// SPDX-License-Identifier: MIT

//! 10xer's keyboard folder peek in Icons and List, on the offscreen host.

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, hosted_after, send_key,
        view::{all, text},
    },
};

const MODULE: &str = "peek::tests";

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

fn peek_shown(host: &Offscreen) -> bool {
    let tree = host.tree();
    !all(&tree, &|node| text(node, "name") == "Folder peek").is_empty()
}

fn open(host: &Offscreen) -> bool {
    host.app().peek.state.target().is_some()
}

/// A 10xer window in `mode` on a folder holding `documents` (two files) and `readme.md`,
/// with the cursor on the folder at `index`.
fn on_folder(host: &mut Offscreen, index: usize) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("folder");
    std::fs::create_dir(root.path().join("documents")).expect("folder");
    for name in ["a.txt", "b.txt"] {
        std::fs::write(root.path().join("documents").join(name), name).expect("file");
    }
    std::fs::write(root.path().join("readme.md"), "readme").expect("file");
    host.app().open(Location::local(root.path()));
    assert!(host.run_until(PATIENCE, |h| {
        h.app().browser().column_entry_counts(0).is_some()
    }));
    while host.turn() {}
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
    while host.turn() {}
    host.app().browser().select(0, index);
    while host.turn() {}
    root
}

fn in_mode(mode: &'static str, name: &str, run: impl FnOnce(Offscreen)) {
    hosted_after(
        MODULE,
        name,
        move || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = format!("browser_mode = \"{mode}\"\ntenxer_mode = true\n");
            std::fs::write(file, saved).expect("settings");
        },
        run,
    );
}

fn i_toggles_the_peek_and_escape_closes_it_first(mode: &'static str, name: &str) {
    in_mode(mode, name, |mut host| {
        let _root = on_folder(&mut host, 0);
        press(&mut host, Key::Text("i".to_owned()));
        assert!(
            open(&host) && peek_shown(&host),
            "i opens the folder's peek"
        );
        let listed = host.run_until(PATIENCE, |h| h.app().peek.entries.len() == 2);
        assert!(listed, "the browser lists the peeked folder");
        press(&mut host, Key::Text("i".to_owned()));
        assert!(!open(&host) && !peek_shown(&host), "i again closes it");

        press(&mut host, Key::Text("i".to_owned()));
        assert!(open(&host));
        press(&mut host, Key::Escape);
        assert!(!open(&host), "Escape closes the peek first");
        assert_eq!(host.app().browser().selected_positions(0), vec![0]);

        press(&mut host, Key::Text("i".to_owned()));
        assert!(open(&host));
        host.app().browser().close_peek();
        while host.turn() {}
        assert!(!open(&host), "a close from the browser closes the card");
    });
}

#[test]
fn list_i_toggles_the_peek_and_escape_closes_it_first() {
    i_toggles_the_peek_and_escape_closes_it_first(
        "list",
        "list_i_toggles_the_peek_and_escape_closes_it_first",
    );
}

#[test]
fn icons_i_toggles_the_peek_and_escape_closes_it_first() {
    i_toggles_the_peek_and_escape_closes_it_first(
        "icons",
        "icons_i_toggles_the_peek_and_escape_closes_it_first",
    );
}

/// On a file `i` is the preview's, and in Columns a folder's column opens: neither peeks.
#[test]
fn i_peeks_only_folders_in_icons_and_list() {
    in_mode(
        "list",
        "i_peeks_only_folders_in_icons_and_list",
        |mut host| {
            let _root = on_folder(&mut host, 1);
            press(&mut host, Key::Text("i".to_owned()));
            assert!(!open(&host), "a file has no peek");
        },
    );
    in_mode(
        "columns",
        "i_peeks_only_folders_in_icons_and_list",
        |mut host| {
            let _root = on_folder(&mut host, 0);
            press(&mut host, Key::Text("i".to_owned()));
            assert!(!open(&host), "Columns never peeks");
        },
    );
}
