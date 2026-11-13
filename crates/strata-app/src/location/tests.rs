// SPDX-License-Identifier: MIT

//! The bar on the offscreen host: where the keyboard goes when an edit ends.

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    commands::{self, On, Outcome},
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, folder_of, hosted, send_key, view},
};

const MODULE: &str = "location::tests";

fn act<T>(host: &mut Offscreen, act: impl FnOnce(&mut App, &mut Tx) -> T) -> T {
    let mut said = None;
    host.runtime().update(|app, tx| said = Some(act(app, tx)));
    idle(host);
    said.expect("the update ran")
}

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn focused(host: &mut Offscreen) -> Option<String> {
    act(host, |_, tx| tx.focused().map(str::to_owned))
}

/// Opens a folder and waits until the bar names it.
fn open(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = folder_of(&["one.txt"]);
    host.app().open(Location::local(folder.path()));
    let named = host.run_until(PATIENCE, |host| host.app().location.shown.is_some());
    assert!(named, "the bar never named the folder");
    idle(host);
    folder
}

/// Ctrl+L's command, and the entry with the keyboard.
fn edit(host: &mut Offscreen) {
    let ran = act(host, |app, tx| {
        commands::run(app, "location", On::Focus, tx)
    });
    assert_eq!(ran, Outcome::Ran);
    let state = &host.app().location.state;
    assert!(state.entry_focused(), "the entry did not take the keyboard");
}

#[test]
fn a_press_elsewhere_leaves_the_keyboard_there_and_escape_gives_it_to_the_view() {
    hosted(
        MODULE,
        "a_press_elsewhere_leaves_the_keyboard_there_and_escape_gives_it_to_the_view",
        |mut host| {
            let _folder = open(&mut host);

            edit(&mut host);
            act(&mut host, |_, tx| tx.focus("header-search", true));
            assert!(!host.app().location.state.editing(), "the edit stayed open");
            assert_eq!(focused(&mut host).as_deref(), Some("header-search"));

            edit(&mut host);
            let escape = KeyEvent {
                key: Key::Escape,
                mods: Mods::default(),
            };
            send_key(&mut host, escape);
            idle(&mut host);
            assert!(
                !host.app().location.state.editing(),
                "Escape left the edit open"
            );
            let view = shell::view_focus(host.app()).map(str::to_owned);
            assert_eq!(focused(&mut host), view);
        },
    );
}

/// The name of the first dialog in the tree.
fn dialog_name(node: &serde_json::Value) -> Option<String> {
    if node["role"] == "dialog" {
        return node["name"].as_str().map(str::to_owned);
    }
    let children = node["children"].as_array().map_or(&[][..], Vec::as_slice);
    children.iter().find_map(dialog_name)
}

/// `submit_location`: text the browser refuses at once is reported, and the keyboard is
/// given back to the view first, so it is not left nowhere under the dialog.
#[test]
fn a_typed_location_refused_at_once_is_reported() {
    hosted(
        MODULE,
        "a_typed_location_refused_at_once_is_reported",
        |mut host| {
            let _folder = open(&mut host);
            edit(&mut host);
            act(&mut host, |app, _| {
                let mut typed = rune::TextEdit::new("not/absolute");
                typed.focused = true;
                app.location.state.edit = typed;
            });

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Enter,
                    mods: Mods::default(),
                },
            );

            let reported = host.run_until(PATIENCE, |host| dialog_name(&host.tree()).is_some());
            assert!(reported, "no dialog");
            let name = dialog_name(&host.tree());
            assert_eq!(name.as_deref(), Some("Unable to open location"));
            // The dialog holds the keyboard; it goes back where it was, the view, once closed.
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            let closed = host.run_until(PATIENCE, |host| dialog_name(&host.tree()).is_none());
            assert!(closed, "the dialog stayed");
            idle(&mut host);
            let view = shell::view_focus(host.app()).map(str::to_owned);
            assert_eq!(focused(&mut host), view, "the keyboard back on the view");
        },
    );
}

/// Presses at the middle of the element keyed `key`.
fn click(host: &mut Offscreen, key: &str) {
    let elements = host.runtime().elements();
    let target = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = target.unwrap_or_else(|| panic!("{key} is drawn")).visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

/// A press anywhere on the strip but a crumb opens the entry, as on GTK, whose strip takes any
/// release a crumb button does not: between two crumbs, on a separator, on the current crumb
/// and on the empty space after it. Copy path, inside the current crumb, takes its own.
#[test]
fn a_press_on_the_strip_but_on_a_crumb_opens_the_entry() {
    hosted(
        MODULE,
        "a_press_on_the_strip_but_on_a_crumb_opens_the_entry",
        |mut host| {
            let _folder = open(&mut host);
            let tree = host.tree();
            let ancestors = view::all(&tree, &|node| {
                view::text(node, "key").starts_with("location-crumb-")
            });
            let current = format!("location-item-{}", ancestors.len());
            let keys = [
                "location-gap-1",
                "location-sep-1",
                current.as_str(),
                "location-crumbs-space",
            ];
            for key in keys {
                click(&mut host, key);
                let state = &host.app().location.state;
                assert!(state.entry_focused(), "a press on {key} opened the entry");
                send_key(
                    &mut host,
                    KeyEvent {
                        key: Key::Escape,
                        mods: Mods::default(),
                    },
                );
                idle(&mut host);
                assert!(
                    !host.app().location.state.entry_focused(),
                    "Escape closed it"
                );
            }

            click(&mut host, "location-copy");
            let state = &host.app().location.state;
            assert!(!state.entry_focused(), "Copy path takes its own press");
        },
    );
}
