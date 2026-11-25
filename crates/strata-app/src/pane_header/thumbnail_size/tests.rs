// SPDX-License-Identifier: MIT

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

        press(&mut host, Key::Escape);
        assert!(!drawn(&host, SLIDER), "Escape closed the popover");
        assert_eq!(focused(&mut host).as_deref(), Some(BUTTON));
    });
}

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
        let tree = host.tree();
        for (key, name) in [
            (MAKE_DEFAULT, "Make this the default size"),
            (RESET, "Reset to default size"),
        ] {
            let action = node(&tree, key).expect("the action");
            assert_eq!(view::text(action, "role"), "menu-item", "{name}, as GTK's");
            assert_eq!(view::text(action, "name"), name);
        }
        return_on(&mut host, MAKE_DEFAULT);
        wait_closed(&mut host, "Make this the default size");
        assert_eq!(default_size(&host), 80);
        assert_eq!(
            own_size(&host, folder.path()),
            None,
            "the default pruned it"
        );
        let default_step = host.app().icons.synced_step();

        open_popover(&mut host);
        for _ in 0..5 {
            press(&mut host, Key::Right);
        }
        let offered = host.run_until(PATIENCE, |host| {
            size::shown(host.app()) == 160 && drawn(host, RESET)
        });
        assert!(offered, "the actions again at 160");
        assert_ne!(
            host.app().icons.synced_step(),
            default_step,
            "larger tile size uses a larger thumbnail step"
        );

        host.runtime()
            .update(|_: &mut App, tx: &mut Tx| tx.focus(RESET, true));
        idle(&mut host);
        let poll = host.runtime().poll.take();
        send_key(
            &mut host,
            KeyEvent {
                key: Key::Enter,
                mods: Mods::default(),
            },
        );
        host.runtime().frame(1.0 / 60.0);
        host.runtime().settle_all();
        host.runtime().poll = poll;
        assert!(
            !drawn(&host, SLIDER),
            "Reset to default size closed the popover on the press frame"
        );
        assert_eq!(
            size::shown(host.app()),
            80,
            "Reset to default size resized immediately without waiting for poll or save delay"
        );
        assert_eq!(
            host.app().icons.synced_step(),
            default_step,
            "Reset to default size synced the grid immediately on the press turn"
        );
        assert_eq!(default_size(&host), 80);
        assert_eq!(
            own_size(&host, folder.path()),
            None,
            "the folder forgot its own"
        );
    });
}

#[test]
fn another_folder_closes_the_popover() {
    let name = "another_folder_closes_the_popover";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = fixture();
        let root = Location::local(folder.path());
        open(&mut host, ICONS, &folder.path().join("documents"));
        open_popover(&mut host);

        host.app().open(root.clone());
        let up = host.run_until(PATIENCE, |host| {
            host.app().browser().active_location().as_ref() == Some(&root)
        });
        assert!(up, "the parent shows");
        let closed = host.run_until(PATIENCE, |host| !drawn(host, SLIDER));
        assert!(closed, "the popover closed with the folder");
    });
}

#[test]
fn delete_on_the_slider_trashes_nothing() {
    let name = "delete_on_the_slider_trashes_nothing";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = fixture();
        open(&mut host, ICONS, folder.path());
        press(&mut host, Key::End);
        let (selected, _) = view::browser_holds(&host);
        let [chosen] = selected.as_slice() else {
            panic!("End selects the last entry: {selected:?}");
        };
        let chosen = folder.path().join(chosen);
        open_popover(&mut host);

        press(&mut host, Key::Delete);
        for _ in 0..20 {
            host.turn();
        }
        idle(&mut host);

        assert!(chosen.exists(), "{} is still there", chosen.display());
        assert!(drawn(&host, SLIDER), "the popover is still open");
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
