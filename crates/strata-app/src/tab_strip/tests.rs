// SPDX-License-Identifier: MIT

//! The strip on the registered window, where GTK was driven to say what it does: a tab hidden
//! with a rename open commits it, and a child column its mirror was about to open is open when
//! the tab comes back, in that tab only.

use std::{path::Path, time::Duration};

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use super::select;
use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    shell::{
        self,
        tabs::{shown_tab, tabs},
    },
    slots::TabKey,
    test_support::{
        PATIENCE, fixture, hosted, send_key,
        view::{field, idle},
    },
};

const MODULE: &str = "tab_strip::tests";

fn act<T>(host: &mut Offscreen, act: impl FnOnce(&mut App, &mut Tx) -> T) -> T {
    let mut said = None;
    host.runtime().update(|app, tx| said = Some(act(app, tx)));
    idle(host);
    said.expect("the update ran")
}

/// The window at `folder`, and a second tab opened at it with the header's New tab, shown.
/// Returns the first tab.
fn two_tabs_at(host: &mut Offscreen, folder: &Path) -> TabKey {
    let at = Location::local(folder);
    host.app().open(at.clone());
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    idle(host);
    let first = shown_tab(host.app());
    let ran = act(host, |app, tx| run(app, "tab-new", On::Focus, tx));
    assert_eq!(ran, Outcome::Ran);
    assert_eq!(tabs(host.app()).len(), 2, "New tab opened a tab");
    let opened = |host: &Offscreen| {
        host.app().browser().active_location() == Some(at.clone())
            && host.app().browser().column_entry_counts(0).is_some()
    };
    assert!(host.run_until(PATIENCE, opened), "the new tab at the same folder");
    idle(host);
    first
}

fn typed(host: &mut Offscreen, text: &str) {
    for character in text.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
    }
    idle(host);
}

/// GTK's rename field commits when the keyboard leaves it, and a tab switch takes the
/// keyboard: the name typed is the entry's, and the tab shown again has no edit open.
#[test]
fn a_rename_open_when_its_tab_is_hidden_is_committed() {
    hosted(
        MODULE,
        "a_rename_open_when_its_tab_is_hidden_is_committed",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let todo = Location::local(folder.path().join("todo.txt"));
            act(&mut host, |app, tx| {
                let row = On::Row((0, shell::id_of(app, &todo)));
                assert_eq!(run(app, "rename", row, tx), Outcome::Ran);
            });
            assert!(field(&host).is_some(), "the editor opened");
            typed(&mut host, "zz");

            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            let renamed = folder.path().join("zz.txt");
            let committed = host.run_until(PATIENCE, |_| renamed.exists());
            assert!(committed, "the name typed was committed");
            assert!(!folder.path().join("todo.txt").exists());

            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            assert_eq!(field(&host), None, "the tab shown again has no edit open");
        },
    );
}

/// A cursor moved onto a folder starts the Columns mirror. A tab hidden while it waits opens
/// no column in the tab shown, and has the folder's column open when it is shown again.
#[test]
fn a_mirror_due_in_a_hidden_tab_opens_its_column_in_that_tab_alone() {
    hosted(
        MODULE,
        "a_mirror_due_in_a_hidden_tab_opens_its_column_in_that_tab_alone",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let documents = Location::local(folder.path().join("documents"));
            assert_eq!(host.app().browser().location_at(1), None, "no child column yet");

            let focus = shell::view_focus(host.app()).expect("the view's key");
            act(&mut host, |_, tx| tx.focus(focus, true));
            // The switch comes before the mirror's wait is over, or just after it.
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Down,
                    mods: Mods::default(),
                },
            );
            // One frame hands the key to the view and its cursor to the browser.
            host.turn();
            let moved = host.app().browser().focused_item().map(|(depth, at, _)| (depth, at));
            assert_eq!(moved, Some((0, 1)), "Down moved the cursor onto documents");
            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            host.run_until(Duration::from_millis(400), |_| false);
            let shown = host.app().browser().location_at(1);
            assert_eq!(shown, None, "the tab shown opened no column");

            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            let opened =
                |host: &Offscreen| host.app().browser().location_at(1) == Some(documents.clone());
            let mirrored = host.run_until(PATIENCE, opened);
            assert!(mirrored, "the mirror opened documents in its own tab");
        },
    );
}

/// What has the keyboard, as an update reads it.
fn keyboard(host: &mut Offscreen) -> Option<String> {
    act(host, |_, tx| tx.focused().map(str::to_owned))
}

/// Presses the strip's tab at `index` with the pointer.
fn click_tab(host: &mut Offscreen, index: usize) {
    let tree = host.tree();
    let tabs = crate::test_support::view::all(&tree, &|node| node["role"] == "tab");
    let bounds = &tabs.get(index).expect("the tab")["bounds"];
    let at = |i: usize| bounds[i].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let (x, y) = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) / 2.0) as f32);
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    idle(host);
}

/// GTK's `select` keeps a tab's focus only in its files, its sidebar or its preview: a tab
/// left by a press on the strip, which took the keyboard, gets its view's back when it is
/// shown again.
#[test]
fn a_tab_left_by_a_press_on_the_strip_gets_the_keyboard_back_in_its_view() {
    hosted(
        MODULE,
        "a_tab_left_by_a_press_on_the_strip_gets_the_keyboard_back_in_its_view",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let view = shell::view_focus(host.app()).expect("the view's key");
            assert_eq!(keyboard(&mut host).as_deref(), Some(view));

            click_tab(&mut host, 0);
            assert_eq!(shown_tab(host.app()), first, "the press showed the first tab");
            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            assert_eq!(keyboard(&mut host).as_deref(), Some(view));
            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            assert_eq!(keyboard(&mut host).as_deref(), Some(view), "the first tab's view");
        },
    );
}
