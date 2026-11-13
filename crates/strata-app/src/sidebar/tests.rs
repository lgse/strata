// SPDX-License-Identifier: MIT

//! The sidebar in the registered window: what assistive technology hears of its rows, what a
//! click and Return open, where the keyboard goes in and out, and the pins the shared store
//! gives it.

use std::path::{Path, PathBuf};

use rune::{Key, Mods};
use serde_json::Value;
use strata_components::sidebar::ItemId;
use strata_core::model::Location;
use strata_fs::adapters::bookmarks;

use crate::{
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, fixture, hosted, hosted_after,
        view::{all, browser_holds, idle, press, text},
    },
};

const MODULE: &str = "sidebar::tests";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("the test's home"))
}

fn keyed<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    all(tree, &|node| node["key"] == key).into_iter().next()
}

fn named_buttons(host: &Offscreen) -> Vec<String> {
    let tree = host.tree();
    let rows = all(&tree, &|node| {
        node["role"] == "button" && text(node, "key").starts_with("sidebar-")
    });
    rows.iter().map(|row| text(row, "name")).collect()
}

/// Waits for the sidebar's row keyed `key`.
fn shown(host: &mut Offscreen, key: &str) -> Value {
    let reached = host.run_until(PATIENCE, |host| keyed(&host.tree(), key).is_some());
    assert!(
        reached,
        "waited for {key}; the sidebar shows {:?}",
        named_buttons(host)
    );
    keyed(&host.tree(), key).expect("the row").clone()
}

/// The key of the element that has the keyboard, as the accessibility tree says.
fn focused(node: &Value) -> Option<String> {
    let states = node["states"].as_array();
    if states.is_some_and(|states| states.iter().any(|state| state == "focused")) {
        return node["key"].as_str().map(str::to_owned);
    }
    let children = node["children"].as_array().into_iter().flatten();
    children.filter_map(focused).next()
}

fn click(host: &mut Offscreen, key: &str) {
    shown(host, key);
    let elements = host.runtime().elements();
    let row = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = row.expect("the row is mounted").visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn wait_for_location(host: &mut Offscreen, folder: &Path) {
    let wanted = Location::local(folder);
    let reached = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location() == Some(wanted.clone())
    });
    assert!(
        reached,
        "waited for the browser to open {}",
        folder.display()
    );
    idle(host);
}

fn opened(folder: &Path, host: &mut Offscreen) {
    host.app().open(Location::local(folder));
    wait_for_location(host, folder);
}

#[test]
fn places_are_buttons_named_and_described_as_gtk_names_its_rows() {
    hosted(
        MODULE,
        "places_are_buttons_named_and_described_as_gtk_names_its_rows",
        |mut host| {
            let row = shown(&mut host, "sidebar-place-home");
            let trash = shown(&mut host, "sidebar-place-trash");
            assert_eq!(text(&row, "role"), "button");
            assert_eq!(text(&row, "name"), "Home");
            let home_path = Location::local(home()).display_path();
            assert_eq!(text(&row, "description"), home_path);
            assert_eq!(text(&trash, "role"), "button");
            assert_eq!(text(&trash, "name"), "Trash");
            assert_eq!(text(&trash, "description"), "trash:///");
            let names = named_buttons(&host);
            let places = &names[..2];
            assert_eq!(places, ["Home", "Trash"], "the default order: {names:?}");
        },
    );
}

#[test]
fn a_click_opens_a_place_and_marks_its_row() {
    hosted(
        MODULE,
        "a_click_opens_a_place_and_marks_its_row",
        |mut host| {
            let folder = fixture();
            opened(folder.path(), &mut host);
            shown(&mut host, "sidebar-place-home");
            assert_eq!(host.app().sidebar.current, None, "the fixture has no row");

            click(&mut host, "sidebar-place-home");
            wait_for_location(&mut host, &home());
            assert_eq!(
                host.app().sidebar.current,
                Some(ItemId::Place("home".into()))
            );
            let (selected, _) = browser_holds(&host);
            assert!(selected.is_empty(), "a click selects nothing: {selected:?}");
        },
    );
}

#[test]
fn the_keyboard_comes_in_at_the_views_start_and_return_opens_a_place_with_its_first_item() {
    hosted(
        MODULE,
        "the_keyboard_comes_in_at_the_views_start_and_return_opens_a_place_with_its_first_item",
        |mut host| {
            for name in ["a.txt", "b.txt"] {
                std::fs::write(home().join(name), name).expect("a file at home");
            }
            let folder = fixture();
            opened(folder.path(), &mut host);
            shown(&mut host, "sidebar-place-home");

            press(&mut host, Key::Down, Mods::default());
            press(&mut host, Key::Left, Mods::default());
            assert_eq!(
                focused(&host.tree()).as_deref(),
                Some("sidebar-place-home"),
                "Left in the first column hands the keyboard to the sidebar's first row"
            );

            press(&mut host, Key::Enter, Mods::default());
            wait_for_location(&mut host, &home());
            let reached = host.run_until(PATIENCE, |host| browser_holds(host).0 == ["a.txt"]);
            assert!(
                reached,
                "Return opens Home with its first item selected: {:?}",
                browser_holds(&host)
            );
            assert_eq!(
                host.app().sidebar.current,
                Some(ItemId::Place("home".into()))
            );
        },
    );
}

fn bookmarks_file() -> PathBuf {
    bookmarks::pinned_places_path()
}

fn pin(path: &Path, name: &str) -> String {
    format!("file://{} {name}\n", path.display())
}

#[test]
fn pins_come_from_the_bookmarks_file_and_follow_the_shared_store() {
    hosted_after(
        MODULE,
        "pins_come_from_the_bookmarks_file_and_follow_the_shared_store",
        || {
            let file = bookmarks_file();
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("gtk-3.0");
            let pins = format!(
                "{}{}",
                pin(&home(), "Home again"),
                pin(Path::new("/srv/work"), "Work")
            );
            std::fs::write(file, pins).expect("the bookmarks file");
        },
        |mut host| {
            let work = shown(&mut host, "sidebar-pin-1");
            assert_eq!(text(&work, "role"), "button");
            assert_eq!(text(&work, "name"), "Work");
            let names = named_buttons(&host);
            assert!(
                !names.iter().any(|name| name == "Home again"),
                "a pin of a built-in place is not listed twice: {names:?}"
            );

            let other = pin(Path::new("/srv/other"), "Other");
            glib::MainContext::default().spawn_local(async move {
                let written = bookmarks::update(move |_| Some(other.clone().into_bytes())).await;
                written.expect("the bookmarks file written");
            });
            let reached = host.run_until(PATIENCE, |host| {
                let names = named_buttons(host);
                names.iter().any(|name| name == "Other") && !names.iter().any(|name| name == "Work")
            });
            assert!(
                reached,
                "the store's change reaches the sidebar: {:?}",
                named_buttons(&host)
            );
        },
    );
}
