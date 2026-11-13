// SPDX-License-Identifier: MIT

//! Return on a focused button presses it, as on GTK: the pane header's Parent folder, and an
//! unreadable folder's Retry. `guard` checks that every other button takes it too.

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

mod guard;

const MODULE: &str = "buttons::tests";

/// The keys of the buttons built through `on_return` or `taking_return` in this process.
static TAKING_RETURN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

pub(super) fn taking_return(key: &str) {
    TAKING_RETURN
        .lock()
        .expect("the keys")
        .insert(key.to_owned());
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

/// C6's own buttons take Return in the component (`guard`'s `IN_THEIR_COMPONENT`): Copy path
/// shows that it copied, and the parent's crumb opens the parent.
#[test]
fn return_presses_copy_path_and_a_crumb() {
    let name = "return_presses_copy_path_and_a_crumb";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = fixture();
        let root = Location::local(folder.path());
        host.app()
            .open(Location::local(folder.path().join("documents")));
        let opened = host.run_until(PATIENCE, |host| {
            let tree = host.tree();
            !view::all(&tree, &|node| node["key"] == "location-copy").is_empty()
        });
        assert!(opened, "documents");
        idle(&mut host);

        return_on(&mut host, "location-copy");
        let copied = host.run_until(PATIENCE, |host| {
            let tree = host.tree();
            let copy = view::all(&tree, &|node| node["key"] == "location-copy");
            copy.first()
                .is_some_and(|node| view::text(node, "name") == "Path copied")
        });
        assert!(copied, "Return on Copy path copied it");

        let tree = host.tree();
        let crumbs = view::all(&tree, &|node| {
            view::text(node, "key").starts_with("location-crumb-")
        });
        let parent = view::text(crumbs.last().expect("an ancestor's crumb"), "key");
        return_on(&mut host, Box::leak(parent.into_boxed_str()));
        let up = host.run_until(PATIENCE, |host| {
            host.app().browser().active_location().as_ref() == Some(&root)
        });
        assert!(up, "Return on the parent's crumb opened it");
    });
}
