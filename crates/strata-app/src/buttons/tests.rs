// SPDX-License-Identifier: MIT

//! Return on a focused button presses it, as on GTK: the pane header's Parent folder, and an
//! unreadable folder's Retry. And every button the keyboard can reach in each view takes
//! Return, so the next button added cannot forget it.

use std::{collections::BTreeSet, os::unix::fs::PermissionsExt, sync::Mutex};

use rune::{Key, KeyEvent, Mods};
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, fixture, send_key,
        view::{self, hosted_in},
    },
};

const MODULE: &str = "buttons::tests";

/// The keys of the buttons built through `on_return` or `taking_return` in this process.
static TAKING_RETURN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

pub(super) fn taking_return(key: &str) {
    TAKING_RETURN
        .lock()
        .expect("the keys")
        .insert(key.to_owned());
}

/// Roles whose own keys move among and press their items, Return included.
const ITEM_GROUPS: [&str; 5] = ["list", "menu", "tree", "grid", "dialog"];

/// Buttons left to Rune's Return for now, each with why.
const EXCEPTIONS: [&str; 3] = [
    // The window's own buttons (C5's `window_buttons`): GTK's likely take no focus, so whether
    // these stop being Tab stops or take Return is the owner's (`owner.md`, A5).
    "header-minimize",
    "header-maximize",
    "header-close",
];

/// The focusable buttons of the window that neither take Return themselves nor sit in a
/// group or dialog that handles it, nor are named in [`EXCEPTIONS`]. The window is built
/// again first, so the keys recorded are those of the buttons drawn now, not of an earlier
/// build under the same key (Back without a history takes [`super::on_return`]).
fn deaf_to_return(host: &mut Offscreen) -> Vec<String> {
    TAKING_RETURN.lock().expect("the keys").clear();
    host.runtime().update(|_: &mut App, _| {});
    idle(host);
    let elements = host.runtime().elements();
    let taking = TAKING_RETURN.lock().expect("the keys");
    let in_group = |mut at: Option<usize>| {
        while let Some(index) = at {
            if ITEM_GROUPS.contains(&elements[index].role) {
                return true;
            }
            at = elements[index].parent;
        }
        false
    };
    elements
        .iter()
        .filter(|element| element.role == "button" && element.focusable)
        .filter(|element| !element.inert && !element.exiting)
        .filter(|element| !in_group(element.parent))
        .map(|element| element.key.clone().unwrap_or_else(|| element.path.clone()))
        .filter(|key| !taking.contains(key) && !EXCEPTIONS.contains(&key.as_str()))
        .collect()
}

/// Opens a folder and then one inside it in `mode`'s view, so Back has a history, with the
/// sidebar shown, and checks every button.
fn every_button_takes_return(mode: &'static str, name: &str) {
    hosted_in(MODULE, mode, name, |mut host| {
        let folder = fixture();
        for opened in [folder.path().to_owned(), folder.path().join("documents")] {
            let location = Location::local(&opened);
            host.app().open(location.clone());
            let shown = host.run_until(PATIENCE, |host| {
                host.app().browser().active_location().as_ref() == Some(&location)
            });
            assert!(shown, "{}", opened.display());
        }
        assert!(host.app().browser().can_go_back(), "Back has a history");
        let sidebar = host.run_until(PATIENCE, |host| {
            let tree = host.tree();
            !view::all(&tree, &|node| node["key"] == "sidebar-place-home").is_empty()
        });
        assert!(sidebar, "the sidebar is shown");
        idle(&mut host);

        let deaf = deaf_to_return(&mut host);
        assert!(
            deaf.is_empty(),
            "buttons that Return does not press; give each `buttons::on_return`: {deaf:?}"
        );
    });
}

#[test]
fn every_button_of_the_columns_view_takes_return() {
    every_button_takes_return("columns", "every_button_of_the_columns_view_takes_return");
}

#[test]
fn every_button_of_the_icons_view_takes_return() {
    every_button_takes_return("icons", "every_button_of_the_icons_view_takes_return");
}

#[test]
fn every_button_of_the_list_view_takes_return() {
    every_button_takes_return("list", "every_button_of_the_list_view_takes_return");
}

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

/// Return on the element keyed `key`, with the keyboard given to it first.
fn return_on(host: &mut Offscreen, key: &'static str) {
    host.runtime()
        .update(move |_: &mut App, tx| tx.focus(key, true));
    idle(host);
    send_key(
        host,
        KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        },
    );
    idle(host);
}

#[test]
fn return_presses_the_pane_headers_parent_folder() {
    let name = "return_presses_the_pane_headers_parent_folder";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = fixture();
        let root = Location::local(folder.path());
        host.app()
            .open(Location::local(folder.path().join("documents")));
        let opened = host.run_until(PATIENCE, |host| {
            host.app().browser().active_location().as_ref() != Some(&root)
                && host.app().browser().active_location().is_some()
        });
        assert!(opened, "documents");
        idle(&mut host);

        return_on(&mut host, "icons-up");

        let up = host.run_until(PATIENCE, |host| {
            host.app().browser().active_location().as_ref() == Some(&root)
        });
        assert!(up, "Return on Parent folder opened the parent");
    });
}

#[test]
fn return_presses_an_unreadable_folders_retry() {
    let name = "return_presses_an_unreadable_folders_retry";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = tempfile::tempdir().expect("a folder");
        let locked = folder.path().join("locked");
        std::fs::create_dir(&locked).expect("the folder");
        std::fs::write(locked.join("inside.txt"), "inside").expect("a file");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        if std::fs::read_dir(&locked).is_ok() {
            eprintln!("skipped: {} stays readable to this user", locked.display());
            return;
        }
        host.app().open(Location::local(&locked));
        let failed = host.run_until(PATIENCE, |host| {
            let snapshot = host.app().browser().active_depth();
            snapshot
                .and_then(|depth| host.app().browser().column_snapshot(depth))
                .is_some_and(|column| column.error.is_some())
        });
        assert!(failed, "the folder failed to list");
        idle(&mut host);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let depth = host.app().browser().active_depth().expect("a column");
        let retry: &'static str = Box::leak(format!("surface-retry-{depth}").into_boxed_str());
        return_on(&mut host, retry);

        let listed = host.run_until(PATIENCE, |host| {
            host.app().browser().with_column_entries(depth, <[_]>::len) == Some(1)
        });
        assert!(listed, "Return on Retry read the folder again");
    });
}
