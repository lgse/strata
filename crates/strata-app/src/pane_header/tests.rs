// SPDX-License-Identifier: MIT

//! The header's buttons on the offscreen host, in the Icons view: each goes where GTK's does
//! and is greyed where the browser cannot go, and a press held on Back that listed the
//! history leaves the next press of any button alone, by the pointer or the keyboard.

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, send_key, view::hosted_in},
};

const MODULE: &str = "pane_header::tests";

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

/// The node keyed `key` in the accessibility tree.
fn node<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    if tree["key"] == key {
        return Some(tree);
    }
    let children = tree["children"].as_array().map_or(&[][..], Vec::as_slice);
    children.iter().find_map(|child| node(child, key))
}

fn greyed(host: &Offscreen, key: &str) -> bool {
    let tree = host.tree();
    let states = node(&tree, key).map(|node| node["states"].clone());
    states
        .and_then(|states| states.as_array().cloned())
        .is_some_and(|states| states.iter().any(|state| state == "disabled"))
}

fn centre(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let elements = host.runtime().elements();
    let button = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = button.expect("the button is drawn").visible;
    (x + w / 2.0, y + h / 2.0)
}

fn click(host: &mut Offscreen, key: &str) {
    idle(host);
    let (x, y) = centre(host, key);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn shows(host: &mut Offscreen, folder: &Location, after: &str) {
    let reached = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location().as_ref() == Some(folder)
    });
    assert!(reached, "after {after}, the view shows {folder:?}");
    idle(host);
}

#[test]
fn each_button_goes_where_gtks_does_and_a_held_back_leaves_the_next_click_alone() {
    let name = "each_button_goes_where_gtks_does_and_a_held_back_leaves_the_next_click_alone";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = fixture();
        let root = Location::local(folder.path());
        let parent = Location::local(folder.path().parent().expect("a parent"));
        let documents = Location::local(folder.path().join("documents"));
        host.app().open(documents.clone());
        shows(&mut host, &documents, "opening documents");
        assert!(greyed(&host, "icons-back"), "Back with no history");
        assert!(greyed(&host, "icons-forward"), "Forward with nothing ahead");

        click(&mut host, "icons-up");
        shows(&mut host, &root, "Parent folder");
        click(&mut host, "icons-back");
        shows(&mut host, &documents, "Back");
        assert!(greyed(&host, "icons-back"), "Back at the history's start");
        click(&mut host, "icons-forward");
        shows(&mut host, &root, "Forward");
        assert!(
            greyed(&host, "icons-forward"),
            "Forward at the history's end"
        );

        // A press held on Back lists the history; Escape closes it without going anywhere.
        let (x, y) = centre(&mut host, "icons-back");
        host.runtime().pointer_move(x, y);
        host.runtime().pointer_down(x, y);
        let listed = host.run_until(PATIENCE, |host| host.app().pane_header.menu.is_open());
        assert!(listed, "the held press listed the history");
        // Let go off Back, as onto the menu's scrim: Back hears no release.
        host.runtime().pointer_move(x, y + 200.0);
        host.runtime().pointer_up(x, y + 200.0);
        idle(&mut host);
        send_key(
            &mut host,
            KeyEvent {
                key: Key::Escape,
                mods: Mods::default(),
            },
        );
        idle(&mut host);
        assert!(!host.app().pane_header.menu.is_open(), "Escape closed it");
        shows(&mut host, &root, "the held press");

        click(&mut host, "icons-up");
        shows(&mut host, &parent, "Parent folder after the held press");
        // Space on the focused Back: no press of the pointer starts anew to clear anything.
        host.runtime()
            .update(|_: &mut crate::App, tx| tx.focus("icons-back", true));
        idle(&mut host);
        send_key(
            &mut host,
            KeyEvent {
                key: Key::Text(" ".to_owned()),
                mods: Mods::default(),
            },
        );
        shows(&mut host, &root, "Space on Back after the held press");
    });
}
