// SPDX-License-Identifier: MIT

//! The footer's count on the window the crate registers.

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, folder_of, hosted, send_key},
};

const MODULE: &str = "footer::tests";

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn count(host: &Offscreen) -> Option<String> {
    super::count::shown(host.app()).map(|count| count.text)
}

#[test]
fn a_selection_counts_its_files_with_the_sizes_of_rows_never_shown() {
    hosted(
        MODULE,
        "a_selection_counts_its_files_with_the_sizes_of_rows_never_shown",
        |mut host| {
            // A short window: a range from the first row to the last leaves rows in the
            // middle that no view ever shows, so only the count asks for their sizes.
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

            press(&mut host, Key::Down, Mods::default());
            press(
                &mut host,
                Key::End,
                Mods {
                    shift: true,
                    ..Mods::default()
                },
            );
            // Each file holds its three-byte name.
            let every_size =
                |host: &Offscreen| count(host).as_deref() == Some("64 files selected (192 B)");
            assert!(
                host.run_until(PATIENCE, every_size),
                "the count has every selected file's size: {:?}",
                count(&host)
            );
        },
    );
}
