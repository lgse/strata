// SPDX-License-Identifier: MIT

//! The Icons header's "Thumbnail size" popover on the offscreen host: its button opens a named
//! slider that sizes the tiles as A1's commands do, kept by the shared policy, and its two
//! actions make the remembered folder's size the default or forget it.

use std::path::Path;

use rune::{Key, KeyEvent, Mods, Tx};
use serde_json::Value;
use strata_core::model::Location;
use strata_desktop::settings::folder_views::key_for_location;

use super::{BUTTON, MAKE_DEFAULT, RESET, SLIDER};
use crate::{
    App, folder_views,
    host::offscreen::Offscreen,
    icons::size,
    shell,
    test_support::{
        PATIENCE, fixture, send_key,
        view::{self, Shape, hosted_in, open},
    },
};

const MODULE: &str = "pane_header::thumbnail_size::tests";

const ICONS: Shape = Shape {
    described: "Icons view",
    list: "grid",
    entry: "grid-cell",
};

const LIST: Shape = Shape {
    described: "List view",
    list: "list",
    entry: "list-item",
};

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn node<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

fn drawn(host: &Offscreen, key: &str) -> bool {
    node(&host.tree(), key).is_some()
}

fn focused(host: &mut Offscreen) -> Option<String> {
    let mut said = None;
    host.runtime()
        .update(|_: &mut App, tx: &mut Tx| said = tx.focused().map(str::to_owned));
    idle(host);
    said
}

fn press(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    idle(host);
}

/// Return on the element keyed `key`, with the keyboard given to it first.
fn return_on(host: &mut Offscreen, key: &'static str) {
    host.runtime()
        .update(move |_: &mut App, tx: &mut Tx| tx.focus(key, true));
    idle(host);
    press(host, Key::Enter);
}

/// Waits for the popover to close: Return presses a button through the Space it sends, which
/// arrives a turn later.
fn wait_closed(host: &mut Offscreen, after: &str) {
    let closed = host.run_until(PATIENCE, |host| !drawn(host, SLIDER));
    assert!(closed, "{after} closed the popover");
}

/// Opens the popover from its button and gives its slider the keyboard.
fn open_popover(host: &mut Offscreen) {
    return_on(host, BUTTON);
    let opened = host.run_until(PATIENCE, |host| drawn(host, SLIDER));
    assert!(opened, "Return on Thumbnail size opened its popover");
    host.runtime()
        .update(|_: &mut App, tx: &mut Tx| tx.focus(SLIDER, true));
    idle(host);
}

fn default_size(host: &Offscreen) -> i32 {
    shell::preferences(host.app()).icons_thumbnail_size
}

/// The size the folder-view store keeps as `folder`'s own.
fn own_size(host: &Offscreen, folder: &Path) -> Option<i32> {
    let key = key_for_location(&Location::local(folder)).expect("a key");
    folder_views::views(host.app()).view(&key).icons_size
}

/// The button is named as GTK's, its slider too, and a step of the slider sizes the tiles
/// and is kept as the remembered folder's own; the slider follows the window's keys while it is
/// open; Escape closes it and gives the button the keyboard back.
#[test]
fn the_slider_sizes_the_tiles_and_escape_closes_it() {
    let name = "the_slider_sizes_the_tiles_and_escape_closes_it";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = fixture();
        open(&mut host, ICONS, folder.path());
        let tree = host.tree();
        let button = node(&tree, BUTTON).expect("the Thumbnail size button");
        assert_eq!(view::text(button, "role"), "button");
        assert_eq!(view::text(button, "name"), "Thumbnail size");

        open_popover(&mut host);
        let tree = host.tree();
        let slider = node(&tree, SLIDER).expect("the slider");
        assert_eq!(view::text(slider, "role"), "slider");
        assert_eq!(view::text(slider, "name"), "Thumbnail size");

        press(&mut host, Key::Right);
        let grew = host.run_until(PATIENCE, |host| size::shown(host.app()) == 80);
        assert!(
            grew,
            "a step of the slider sized the tiles: {}",
            size::shown(host.app())
        );
        assert_eq!(
            default_size(&host),
            64,
            "the remembered folder keeps its own"
        );
        assert_eq!(own_size(&host, folder.path()), Some(80));

        let larger = Mods {
            ctrl: true,
            cmd: true,
            alt: true,
            ..Mods::default()
        };
        send_key(
            &mut host,
            KeyEvent {
                key: Key::Command('='),
                mods: larger,
            },
        );
        let followed = host.run_until(PATIENCE, |host| {
            let tree = host.tree();
            let slider = node(&tree, SLIDER).map(|node| view::text(node, "value"));
            slider.as_deref() == Some("96 px")
        });
        assert!(
            followed,
            "the slider follows Ctrl+Alt+=: {:?}",
            node(&host.tree(), SLIDER)
        );

        press(&mut host, Key::Escape);
        assert!(!drawn(&host, SLIDER), "Escape closed the popover");
        assert_eq!(focused(&mut host).as_deref(), Some(BUTTON));
    });
}

/// "Make this the default size" makes the folder's size the default, and "Reset to default
/// size" makes the folder follow it again; each closes the popover, and neither shows while
/// the folder is at the default.
#[test]
fn make_default_and_reset_follow_the_remembered_folder() {
    let name = "make_default_and_reset_follow_the_remembered_folder";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = fixture();
        open(&mut host, ICONS, folder.path());
        open_popover(&mut host);
        assert!(
            !drawn(&host, MAKE_DEFAULT),
            "no actions at the default size"
        );

        press(&mut host, Key::Right);
        let offered = host.run_until(PATIENCE, |host| drawn(host, MAKE_DEFAULT));
        assert!(offered, "a size other than the default offers the actions");
        return_on(&mut host, MAKE_DEFAULT);
        wait_closed(&mut host, "Make this the default size");
        assert_eq!(default_size(&host), 80);
        assert_eq!(
            own_size(&host, folder.path()),
            None,
            "the default pruned it"
        );

        open_popover(&mut host);
        press(&mut host, Key::Right);
        let offered = host.run_until(PATIENCE, |host| drawn(host, RESET));
        assert!(offered, "the actions again at 96");
        return_on(&mut host, RESET);
        wait_closed(&mut host, "Reset to default size");
        let back = host.run_until(PATIENCE, |host| size::shown(host.app()) == 80);
        assert!(
            back,
            "the folder follows the default: {}",
            size::shown(host.app())
        );
        assert_eq!(default_size(&host), 80);
        assert_eq!(
            own_size(&host, folder.path()),
            None,
            "the folder forgot its own"
        );
    });
}

/// The window's commands run while the popover is open, as under GTK's plain popover: Alt+Up
/// on the slider goes to the parent folder, and the popover goes, as GTK's goes with its pane,
/// leaving the keyboard to the new listing.
#[test]
fn alt_up_goes_to_the_parent_and_closes_the_popover() {
    let name = "alt_up_goes_to_the_parent_and_closes_the_popover";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = fixture();
        let root = Location::local(folder.path());
        open(&mut host, ICONS, &folder.path().join("documents"));
        open_popover(&mut host);

        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        send_key(
            &mut host,
            KeyEvent {
                key: Key::Up,
                mods: alt,
            },
        );
        let up = host.run_until(PATIENCE, |host| {
            host.app().browser().active_location().as_ref() == Some(&root)
        });
        assert!(up, "Alt+Up went to the parent");
        let closed = host.run_until(PATIENCE, |host| !drawn(host, SLIDER));
        assert!(closed, "the popover closed with the folder");
        let view = host.run_until(PATIENCE, |host| {
            shell::view_focus(host.app()).is_some_and(|view| view == crate::icons::VIEW)
        });
        assert!(view, "the Icons view shows");
        assert_eq!(
            focused(&mut host).as_deref(),
            Some(crate::icons::VIEW),
            "the new listing takes the keyboard, as GTK's does from inside the pane"
        );
    });
}

#[test]
fn the_list_view_has_no_thumbnail_size_button() {
    let name = "the_list_view_has_no_thumbnail_size_button";
    hosted_in(MODULE, "list", name, |mut host| {
        let folder = fixture();
        open(&mut host, LIST, folder.path());
        assert!(drawn(&host, "list-back"), "the List header");
        assert!(!drawn(&host, BUTTON));
    });
}
