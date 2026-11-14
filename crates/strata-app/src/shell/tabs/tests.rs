// SPDX-License-Identifier: MIT

//! The window's tabs, in the registered window: each keeps its own browser and state, a hidden
//! one runs nothing until it is shown and holds a bounded queue meanwhile, a setting another
//! tab changed is followed when it is shown, and a tab shown again keeps its scroll, its
//! focused entry, and no location edit.

use std::path::Path;

use rune::{Key, Mods, Tx};
use serde_json::Value;
use strata_core::model::Location;

use super::{HELD_EVENTS, close, move_to, open, show, shown_tab, tab_browser, tabs};
use crate::{
    App,
    commands::{self, On},
    host::offscreen::Offscreen,
    shell::{self, hidden_tabs},
    slots::TabKey,
    test_support::{
        PATIENCE, fixture, folder_of, hosted,
        view::{all, browser_holds, drawn, field, hosted_in, idle, press, text, wheel},
    },
};

const MODULE: &str = "shell::tabs::tests";

fn act<T>(host: &mut Offscreen, act: impl FnOnce(&mut App, &mut Tx) -> T) -> T {
    let mut said = None;
    host.runtime().update(|app, tx| said = Some(act(app, tx)));
    idle(host);
    said.expect("the update ran")
}

fn opened(host: &mut Offscreen, folder: &Path) {
    let wanted = Location::local(folder);
    host.app().open(wanted.clone());
    wait_for(host, &wanted);
}

fn wait_for(host: &mut Offscreen, wanted: &Location) {
    let reached = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location().as_ref() == Some(wanted)
    });
    assert!(reached, "the shown tab opens {wanted:?}");
    idle(host);
}

fn located(host: &Offscreen, key: TabKey) -> Option<Location> {
    tab_browser(host.app(), key).and_then(|browser| browser.active_location())
}

fn named(tree: &Value, name: &str) -> bool {
    !all(tree, &|node| text(node, "name") == name).is_empty()
}

/// Opens a tab after the shown one at `folder`, and waits until it shows it.
fn second_tab(host: &mut Offscreen, folder: &Path) -> TabKey {
    let at = Location::local(folder);
    let key = act(host, |app, tx| open(app, Some(at.clone()), 1, tx));
    wait_for(host, &at);
    key
}

/// A tab opened at a folder has a browser of its own; showing either tab gives the window its
/// browser and leaves the other's where it was; closing the shown tab shows its neighbour, and
/// the last tab stays.
#[test]
fn each_tab_keeps_its_own_browser_and_a_shown_tab_takes_the_window() {
    hosted(
        MODULE,
        "each_tab_keeps_its_own_browser_and_a_shown_tab_takes_the_window",
        |mut host| {
            let folder = fixture();
            let documents = folder.path().join("documents");
            let archive = folder.path().join("archive");
            opened(&mut host, &documents);
            let first = shown_tab(host.app());
            let second = second_tab(&mut host, &archive);
            assert_eq!(tabs(host.app()), [first, second]);
            assert_eq!(located(&host, first), Some(Location::local(&documents)));

            assert!(act(&mut host, |app, tx| show(app, first, tx)));
            wait_for(&mut host, &Location::local(&documents));
            assert_eq!(located(&host, second), Some(Location::local(&archive)));
            assert!(
                !act(&mut host, |app, tx| show(app, first, tx)),
                "shown already"
            );

            assert!(act(&mut host, |app, _| move_to(app, first, 9)));
            assert_eq!(tabs(host.app()), [second, first]);
            assert!(act(&mut host, |app, tx| close(app, first, tx)));
            wait_for(&mut host, &Location::local(&archive));
            assert_eq!(tabs(host.app()), [second]);
            assert!(
                !act(&mut host, |app, tx| close(app, second, tx)),
                "the last tab stays"
            );
        },
    );
}

/// What a hidden tab's browser does is held for it, and none of it reaches the shown tab's
/// view. Past the bound its events are dropped, and the tab shown again is rebuilt from its
/// browser, which ends where the events would have.
#[test]
fn a_hidden_tab_runs_nothing_and_holds_a_bounded_queue_until_it_is_shown() {
    hosted(
        MODULE,
        "a_hidden_tab_runs_nothing_and_holds_a_bounded_queue_until_it_is_shown",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let first = shown_tab(host.app());
            second_tab(&mut host, &folder.path().join("documents"));

            let hidden = tab_browser(host.app(), first)
                .expect("the hidden tab")
                .clone();
            // An odd count: the hidden tab ends showing hidden files.
            for _ in 0..=HELD_EVENTS * 2 {
                hidden.toggle_hidden();
            }
            idle(&mut host);
            assert!(
                !named(&host.tree(), ".hidden.txt"),
                "the shown tab heard nothing"
            );
            for (_, accounted) in hidden_tabs(host.app()) {
                assert!(accounted, "nothing ran while hidden");
            }
            for (_, held) in host.app().window.tabs.held() {
                assert!(held.len() <= HELD_EVENTS, "held {} updates", held.len());
                assert!(held.outran(), "the events outran the bound");
            }

            act(&mut host, |app, tx| show(app, first, tx));
            let shown = host.run_until(PATIENCE, |host| named(&host.tree(), ".hidden.txt"));
            assert!(shown, "the tab shown again lists its hidden file");
        },
    );
}

/// A setting another tab changed while this one was hidden is followed when it is shown, as
/// GTK binds it to every tab's browser: here, hidden files.
#[test]
fn a_hidden_tab_follows_a_setting_another_tab_changed() {
    hosted(
        MODULE,
        "a_hidden_tab_follows_a_setting_another_tab_changed",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let first = shown_tab(host.app());
            second_tab(&mut host, folder.path());
            act(&mut host, |app, _| app.browser().toggle_hidden());
            let saved = host.run_until(PATIENCE, |host| shell::preferences(host.app()).show_hidden);
            assert!(saved, "the shown tab saved hidden files shown");

            act(&mut host, |app, tx| show(app, first, tx));
            assert!(host.app().browser().preferences().show_hidden);
            let shown = host.run_until(PATIENCE, |host| named(&host.tree(), ".hidden.txt"));
            assert!(shown, "the first tab lists the hidden file");
        },
    );
}

/// The name of the element that has the keyboard, or of the entry a focused list has active.
fn focused_name(tree: &Value) -> Option<String> {
    let focused = all(tree, &|node| {
        let states = node["states"].as_array();
        states.is_some_and(|states| states.iter().any(|state| state == "focused"))
    });
    let node = focused.last()?;
    let active = &node["activeDescendant"];
    if active.is_string() {
        let entry = all(node, &|entry| entry["key"] == *active);
        return entry.first().map(|entry| text(entry, "name"));
    }
    Some(text(node, "name"))
}

/// Whether the tree shows the last entry and not the first: the view is scrolled to its end.
fn at_the_end(host: &Offscreen) -> bool {
    let drawn = drawn(host);
    drawn.iter().any(|name| name == "file299.txt")
        && !drawn.iter().any(|name| name == "file000.txt")
}

/// A tab shown again is scrolled where it was, which is not where its cursor is, as GTK's
/// hidden tab keeps its scroll; and its cursor is where it was.
fn keeps_its_scroll(mode: &'static str, name: &str) {
    hosted_in(MODULE, mode, name, |mut host| {
        let names: Vec<String> = (0..300).map(|n| format!("file{n:03}.txt")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let folder = folder_of(&names);
        opened(&mut host, folder.path());
        let first = shown_tab(host.app());
        let listed = host.run_until(PATIENCE, |host| named(&host.tree(), "file000.txt"));
        assert!(listed, "the folder is listed");
        press(&mut host, Key::Home, Mods::default());
        wheel(&mut host, -400.0, 40);
        let scrolled = host.run_until(PATIENCE, at_the_end);
        assert!(
            scrolled,
            "the wheel scrolls to the end: {:?}",
            drawn(&host).first()
        );

        second_tab(&mut host, folder.path());
        let fresh = host.run_until(PATIENCE, |host| named(&host.tree(), "file000.txt"));
        assert!(fresh, "the new tab starts at its first entry");

        act(&mut host, |app, tx| show(app, first, tx));
        let back = host.run_until(PATIENCE, at_the_end);
        assert!(back, "the tab is scrolled where it was");
        assert_eq!(
            browser_holds(&host).1.as_deref(),
            Some("file000.txt"),
            "the cursor stays where it was"
        );
    });
}

#[test]
fn a_columns_tab_keeps_its_scroll() {
    keeps_its_scroll("columns", "a_columns_tab_keeps_its_scroll");
}

#[test]
fn a_list_tab_keeps_its_scroll() {
    keeps_its_scroll("list", "a_list_tab_keeps_its_scroll");
}

/// The keyboard goes back where it was in a tab shown again, as GTK's `select` restores the
/// focus a tab saved: here, a row of the sidebar.
#[test]
fn a_tab_shown_again_gives_the_keyboard_back_where_it_was() {
    hosted(
        MODULE,
        "a_tab_shown_again_gives_the_keyboard_back_where_it_was",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let first = shown_tab(host.app());
            press(&mut host, Key::Left, Mods::default());
            let left = focused_name(&host.tree());
            assert!(
                left.as_deref()
                    .is_some_and(|name| !name.ends_with(".txt") && name != "archive"),
                "Left at the view's start gives the sidebar the keyboard: {left:?}"
            );

            second_tab(&mut host, folder.path());
            act(&mut host, |app, tx| show(app, first, tx));
            let back = host.run_until(PATIENCE, |host| focused_name(&host.tree()) == left);
            assert!(
                back,
                "the keyboard is back on {left:?}, not {:?}",
                focused_name(&host.tree())
            );
        },
    );
}

/// A tab left while its location is edited comes back with the edit ended and the keyboard
/// in its files, as GTK's `select` cancels the edit of the tab it leaves.
#[test]
fn a_location_edit_ends_when_its_tab_is_left() {
    hosted(
        MODULE,
        "a_location_edit_ends_when_its_tab_is_left",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let first = shown_tab(host.app());
            act(&mut host, |app, tx| {
                commands::run(app, "location", On::Focus, tx);
            });
            let editing = host.run_until(PATIENCE, |host| {
                field(host).is_some_and(|(_, focused)| focused)
            });
            assert!(editing, "the location is edited");

            second_tab(&mut host, folder.path());
            act(&mut host, |app, tx| show(app, first, tx));
            let ended = host.run_until(PATIENCE, |host| field(host).is_none());
            assert!(ended, "the edit ended: {:?}", field(&host));
            let entries = ["archive", "documents", "pictures", "readme.md", "todo.txt"];
            let files = host.run_until(PATIENCE, |host| {
                focused_name(&host.tree()).is_some_and(|name| entries.contains(&name.as_str()))
            });
            assert!(
                files,
                "the keyboard is in the files: {:?}",
                focused_name(&host.tree())
            );
        },
    );
}

/// Each tab has a sidebar of its own, as GTK builds one for each, and a tab shown again
/// follows what another changed in the settings while it was hidden: here, a place hidden.
#[test]
fn a_tabs_sidebar_follows_a_place_another_tab_hid() {
    hosted(MODULE, "a_tabs_sidebar_follows_a_place_another_tab_hid", |mut host| {
        let folder = fixture();
        opened(&mut host, folder.path());
        let home = |host: &Offscreen| {
            let tree = host.tree();
            !all(&tree, &|node| node["key"] == "sidebar-place-home").is_empty()
        };
        assert!(host.run_until(PATIENCE, home), "the sidebar lists Home");
        let first = shown_tab(host.app());
        second_tab(&mut host, folder.path());
        act(&mut host, |app, _| {
            shell::saved(app).change(|settings| settings.sidebar_show_home = false);
        });

        act(&mut host, |app, tx| show(app, first, tx));
        let gone = host.run_until(PATIENCE, |host| !home(host));
        assert!(gone, "the first tab's sidebar hides Home");
    });
}
