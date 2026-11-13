// SPDX-License-Identifier: MIT

//! BackSpace from the listing goes up a folder in every view, as GTK's `navigate_up` does,
//! and outside the listing it is left alone.

use rune::{Key, Mods};
use strata_core::model::Location;

use super::{MODULE, act, press};
use crate::{
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, fixture, view::hosted_in},
};

/// Opens the fixture's `documents` and gives the shown view the keyboard.
fn in_documents(host: &mut Offscreen) -> (tempfile::TempDir, Location) {
    let folder = fixture();
    let documents = Location::local(folder.path().join("documents"));
    host.app().open(documents.clone());
    let listed = |host: &Offscreen| {
        host.app().browser().active_location().as_ref() == Some(&documents)
            && host.app().browser().column_entry_counts(0).is_some()
    };
    assert!(host.run_until(PATIENCE, listed), "documents' listing");
    act(host, |app, tx| {
        let view = shell::view_focus(app).expect("a shown view");
        tx.focus(view, true);
    });
    (folder, documents)
}

fn backspace(host: &mut Offscreen) {
    press(host, Key::Backspace, Mods::default());
}

fn goes_up_from_the_listing(mode: &'static str, name: &str) {
    hosted_in(MODULE, mode, &format!("navigate_up::{name}"), |mut host| {
        let (folder, documents) = in_documents(&mut host);
        let root = Location::local(folder.path());

        backspace(&mut host);

        let came_from = |host: &Offscreen| {
            let browser = host.app().browser();
            let cursor = browser
                .active_depth()
                .and_then(|depth| browser.cursor_entry(depth));
            browser.active_location().as_ref() == Some(&root)
                && cursor.is_some_and(|entry| entry.location == documents)
        };
        assert!(
            host.run_until(PATIENCE, came_from),
            "BackSpace in {mode} did not open the parent folder at the folder came from"
        );
    });
}

in_each_view! {
    goes_up_from_the_listing:
    "columns" => backspace_in_the_columns_goes_up_to_the_folder_came_from,
    "icons" => backspace_in_the_grid_goes_up_to_the_folder_came_from,
    "list" => backspace_in_the_table_goes_up_to_the_folder_came_from,
}

/// `items_unfocused_for`: away from the listing BackSpace is not the items'.
#[test]
fn backspace_away_from_the_listing_goes_nowhere() {
    hosted_in(
        MODULE,
        "columns",
        "navigate_up::backspace_away_from_the_listing_goes_nowhere",
        |mut host| {
            let (_folder, documents) = in_documents(&mut host);
            act(&mut host, |_, tx| tx.focus("header-search", true));

            backspace(&mut host);
            host.run_until(std::time::Duration::from_millis(300), |_| false);

            let shown = host.app().browser().active_location();
            assert_eq!(shown, Some(documents));
        },
    );
}
