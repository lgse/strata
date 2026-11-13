// SPDX-License-Identifier: MIT

//! The plain popover on the offscreen host, through its first user, the Icons header's
//! "Thumbnail size": each case is one GTK showed when driven (the module docs).

use rune::{GesturePhase, Key, KeyEvent, Mods, Tx, WheelEvent};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, folder_of, send_key,
        view::{self, Shape, browser_holds, hosted_in, top_of_first},
    },
};

const MODULE: &str = "popover::tests";

const ICONS: Shape = Shape {
    described: "Icons view",
    list: "grid",
    entry: "grid-cell",
};

const BUTTON: &str = "icons-thumbnail-size";
const SLIDER: &str = "icons-thumbnail-slider";
const MAKE_DEFAULT: &str = "icons-thumbnail-make-default";
const RESET: &str = "icons-thumbnail-reset";

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

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    idle(host);
}

/// The middle of the element keyed `key`, in window points.
fn centre(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let elements = host.runtime().elements();
    let found = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = found.unwrap_or_else(|| panic!("{key} is drawn")).visible;
    (x + w / 2.0, y + h / 2.0)
}

/// The middle of the tile named `name`.
fn tile(host: &mut Offscreen, name: &str) -> (f32, f32) {
    let tree = host.tree();
    let tiles = view::all(&tree, &|node| {
        node["role"] == ICONS.entry && node["name"] == name
    });
    let key = view::text(tiles.first().expect("the tile"), "key");
    centre(host, &key)
}

fn click_at(host: &mut Offscreen, (x, y): (f32, f32)) {
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

/// A wheel turn down at `(x, y)`, offered to the handlers there as a window host offers it:
/// a gesture of its own, as a user's turns far apart are (headless, Rune's virtual clock would
/// otherwise join it to the last one).
fn wheel_down_at(host: &mut Offscreen, (x, y): (f32, f32)) {
    let runtime = host.runtime();
    runtime.wheel_event(
        WheelEvent::new(0.0, -120.0)
            .at(x, y)
            .phase(GesturePhase::Began),
    );
    runtime.wheel_event(
        WheelEvent::new(0.0, 0.0)
            .at(x, y)
            .phase(GesturePhase::Ended),
    );
    idle(host);
}

/// A folder of two hundred files in the Icons view, enough to scroll, listed and settled.
fn many(host: &mut Offscreen) -> tempfile::TempDir {
    let names: Vec<String> = (1..=200).map(|n| format!("f{n:03}.txt")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let folder = folder_of(&names);
    host.app().open(Location::local(folder.path()));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    let drawn = |host: &Offscreen| view::drawn(host).contains(&"f003.txt".to_owned());
    assert!(host.run_until(PATIENCE, drawn), "the first tiles");
    idle(host);
    folder
}

/// Opens the popover with its button, which gives the slider the keyboard.
fn open_popover(host: &mut Offscreen) {
    let button = centre(host, BUTTON);
    click_at(host, button);
    let opened = host.run_until(PATIENCE, |host| drawn(host, SLIDER));
    assert!(opened, "the popover opened");
    assert_eq!(
        focused(host).as_deref(),
        Some(SLIDER),
        "the slider has the keyboard"
    );
}

fn assert_closed_to_the_button(host: &mut Offscreen, after: &str) {
    let closed = host.run_until(PATIENCE, |host| !drawn(host, SLIDER));
    assert!(closed, "{after} closed the popover");
    assert_eq!(
        focused(host).as_deref(),
        Some(BUTTON),
        "{after} gave the button the keyboard"
    );
}

/// Tab and Shift+Tab move through the popover's controls and wrap; the keyboard stays in it.
#[test]
fn tab_cycles_through_the_popovers_controls() {
    let name = "tab_cycles_through_the_popovers_controls";
    hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = many(&mut host);
        open_popover(&mut host);
        press(&mut host, Key::Right, Mods::default());
        let offered = host.run_until(PATIENCE, |host| drawn(host, MAKE_DEFAULT));
        assert!(offered, "the actions show at another size than the default");

        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        let steps = [
            (Mods::default(), MAKE_DEFAULT),
            (Mods::default(), RESET),
            (Mods::default(), SLIDER),
            (shift, RESET),
            (shift, MAKE_DEFAULT),
        ];
        for (mods, lands) in steps {
            press(&mut host, Key::Tab, mods);
            assert_eq!(focused(&mut host).as_deref(), Some(lands), "{mods:?}");
        }
    });
}

/// A press on a tile outside the popover closes it and selects nothing.
#[test]
fn a_press_on_the_listing_closes_it_and_selects_nothing() {
    let name = "a_press_on_the_listing_closes_it_and_selects_nothing";
    hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = many(&mut host);
        let before = browser_holds(&host).0;
        open_popover(&mut host);

        let third = tile(&mut host, "f003.txt");
        click_at(&mut host, third);

        assert_closed_to_the_button(&mut host, "a press on a tile");
        assert_eq!(browser_holds(&host).0, before, "the press selected nothing");
    });
}

/// A press on the sidebar's Home closes the popover and opens nothing.
#[test]
fn a_press_on_the_sidebar_closes_it_and_opens_nothing() {
    let name = "a_press_on_the_sidebar_closes_it_and_opens_nothing";
    hosted_in(MODULE, "icons", name, |mut host| {
        let folder = many(&mut host);
        open_popover(&mut host);

        let home = centre(&mut host, "sidebar-place-home");
        click_at(&mut host, home);

        assert_closed_to_the_button(&mut host, "a press on Home");
        let shown = host.app().browser().active_location();
        assert_eq!(
            shown,
            Some(Location::local(folder.path())),
            "nothing opened"
        );
    });
}

/// A wheel over the listing closes the popover and scrolls the listing; over the sidebar it
/// closes the popover and scrolls nothing.
#[test]
fn a_wheel_closes_it_and_scrolls_the_listing_only_over_it() {
    let name = "a_wheel_closes_it_and_scrolls_the_listing_only_over_it";
    hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = many(&mut host);
        let top = top_of_first(&host);

        open_popover(&mut host);
        let home = centre(&mut host, "sidebar-place-home");
        wheel_down_at(&mut host, home);
        assert_closed_to_the_button(&mut host, "a wheel over the sidebar");
        assert_eq!(
            top_of_first(&host),
            top,
            "a wheel over the sidebar scrolls nothing"
        );

        open_popover(&mut host);
        let third = tile(&mut host, "f003.txt");
        wheel_down_at(&mut host, third);
        assert_closed_to_the_button(&mut host, "a wheel over the listing");
        let scrolled = host.run_until(PATIENCE, |host| top_of_first(host) != top);
        assert!(scrolled, "a wheel over the listing scrolls it");
    });
}

fn right_click_at(host: &mut Offscreen, (x, y): (f32, f32)) {
    host.runtime().pointer_move(x, y);
    host.runtime().secondary_down(x, y);
    host.runtime().secondary_up(x, y);
    idle(host);
}

/// A right-click outside, on a tile or on the sidebar's Home, closes the popover and opens no
/// menu.
#[test]
fn a_right_click_outside_closes_it_and_opens_no_menu() {
    let name = "a_right_click_outside_closes_it_and_opens_no_menu";
    hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = many(&mut host);
        let before = browser_holds(&host).0;

        open_popover(&mut host);
        let third = tile(&mut host, "f003.txt");
        right_click_at(&mut host, third);
        assert_closed_to_the_button(&mut host, "a right-click on a tile");
        assert_eq!(view::open_menu(&host), None, "no menu opened");
        assert_eq!(
            browser_holds(&host).0,
            before,
            "the right-click selected nothing"
        );

        open_popover(&mut host);
        let home = centre(&mut host, "sidebar-place-home");
        right_click_at(&mut host, home);
        assert_closed_to_the_button(&mut host, "a right-click on Home");
        assert_eq!(view::open_menu(&host), None, "no menu opened");
    });
}

/// Up and Down move through the popover's controls and stop at the ends; on the slider they
/// step it.
#[test]
fn up_and_down_move_through_the_controls_and_step_the_slider() {
    let name = "up_and_down_move_through_the_controls_and_step_the_slider";
    hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = many(&mut host);
        open_popover(&mut host);
        press(&mut host, Key::Right, Mods::default());
        let offered = host.run_until(PATIENCE, |host| drawn(host, MAKE_DEFAULT));
        assert!(offered, "the actions show at another size than the default");
        press(&mut host, Key::Tab, Mods::default());

        let steps = [
            (Key::Down, RESET),
            (Key::Down, RESET),
            (Key::Up, MAKE_DEFAULT),
            (Key::Up, SLIDER),
        ];
        for (key, lands) in steps {
            press(&mut host, key.clone(), Mods::default());
            assert_eq!(focused(&mut host).as_deref(), Some(lands), "{key:?}");
        }
        let size = crate::icons::size::shown(host.app());
        press(&mut host, Key::Up, Mods::default());
        assert_eq!(
            focused(&mut host).as_deref(),
            Some(SLIDER),
            "Up stays on the slider"
        );
        let stepped = host.run_until(PATIENCE, |host| {
            crate::icons::size::shown(host.app()) > size
        });
        assert!(stepped, "Up on the slider steps it");
    });
}

/// The keyboard leaving the popover closes it, as GTK's autohide does.
#[test]
fn the_keyboard_leaving_it_closes_it() {
    hosted_in(
        MODULE,
        "icons",
        "the_keyboard_leaving_it_closes_it",
        |mut host| {
            let _folder = many(&mut host);
            open_popover(&mut host);

            host.runtime()
                .update(|_: &mut App, tx: &mut Tx| tx.focus(crate::icons::VIEW, true));
            idle(&mut host);

            let closed = host.run_until(PATIENCE, |host| !drawn(host, SLIDER));
            assert!(closed, "the popover closed when the keyboard left it");
        },
    );
}
