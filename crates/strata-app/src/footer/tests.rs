// SPDX-License-Identifier: MIT

//! The footer's count on the window the crate registers, read as the window draws it.

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, folder_of, hosted, send_key,
        view::{all, text},
    },
};

const MODULE: &str = "footer::tests";

/// Every selected file's size known: each file holds its three-byte name.
const EVERY_SIZE: &str = "64 files selected (192 B)";

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

/// The count the footer draws.
fn drawn(host: &Offscreen) -> Option<String> {
    let tree = host.tree();
    let count = all(&tree, &|node| node["key"] == "footer-count");
    count.first().map(|node| text(node, "name"))
}

fn counts_every_size(host: &Offscreen) -> bool {
    drawn(host).as_deref() == Some(EVERY_SIZE)
}

/// A short window over a folder and 64 files, with the keyboard on the first file: a range
/// from it to the last row leaves rows in the middle that no view ever shows, so only the
/// count asks for their sizes.
fn short_listing(host: &mut Offscreen) -> tempfile::TempDir {
    host.runtime().set_viewport((800.0, 300.0), 1.0);
    let names: Vec<String> = (0..64).map(|n| format!("f{n:02}")).collect();
    let folder = folder_of(&names.iter().map(String::as_str).collect::<Vec<_>>());
    std::fs::create_dir(folder.path().join("a-folder")).expect("folder");
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
    while host.turn() {}
    press(host, Key::Down, Mods::default());
    folder
}

#[test]
fn a_selection_counts_its_files_with_the_sizes_of_rows_never_shown() {
    hosted(
        MODULE,
        "a_selection_counts_its_files_with_the_sizes_of_rows_never_shown",
        |mut host| {
            let _folder = short_listing(&mut host);
            press(&mut host, Key::End, shift());
            assert!(
                host.run_until(PATIENCE, counts_every_size),
                "the count has every selected file's size: {:?}",
                drawn(&host)
            );
        },
    );
}

#[test]
fn a_reload_while_the_sizes_are_out_asks_for_them_again() {
    hosted(
        MODULE,
        "a_reload_while_the_sizes_are_out_asks_for_them_again",
        |mut host| {
            let _folder = short_listing(&mut host);
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::End,
                    mods: shift(),
                },
            );
            // Two frames and no GLib dispatch: the count has asked, and no fill has landed.
            shell::publish(&mut host.runtime().state);
            host.runtime().frame(0.0);
            host.runtime().frame(0.0);
            host.runtime()
                .update(|app: &mut App, _: &mut Tx| app.browser().reload_active());
            assert!(
                host.run_until(PATIENCE, counts_every_size),
                "the count has every selected file's size after the reload: {:?}",
                drawn(&host)
            );
        },
    );
}
