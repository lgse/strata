// SPDX-License-Identifier: MIT

//! What runs a command on a place, a location that is no row of the view: the sidebar's
//! menu rows and its middle click, and a crumb's middle click, each on the location it was
//! asked on (`On::Place`). `open-in-tab` is the tab strip's own, so each opens a tab there;
//! the others are a made-up part's, which records what it ran.

use std::{cell::RefCell, path::Path};

use rune::{Mods, PhysicalInput, PointerButton, Tx};
use strata_core::model::Location;

use super::super::{Action, Call, On, Outcome};
use super::routing::part;
use crate::{
    App,
    host::offscreen::Offscreen,
    shell::tabs::{shown_tab, tab_browser, tabs},
    slots::{Entry, TabKey},
    test_support::{
        PATIENCE, fixture, hosted_over,
        view::{all, idle, text},
    },
};

const MODULE: &str = "commands::tests::places";

thread_local! {
    /// Each command the made-up part ran, and the place it ran on.
    static RAN: RefCell<Vec<(&'static str, Location)>> = const { RefCell::new(Vec::new()) };
}

fn ran(command: &'static str, call: &Call<'_>) -> Outcome {
    let On::Place(location) = &call.on else {
        return Outcome::Declined;
    };
    RAN.with_borrow_mut(|ran| ran.push((command, location.clone())));
    Outcome::Ran
}

fn in_window(_: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    ran("open-in-window", call)
}

fn customize(_: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    ran("customize", call)
}

const PLACES: &[Action<App>] = &[
    Action::new("open-in-window", in_window),
    Action::new("customize", customize),
];

/// The registered parts, and one that registers the other commands a place's routes name.
fn with_the_commands() -> Vec<Entry<App>> {
    struct Places;
    crate::parts_and(vec![part::<Places>("places", PLACES, |_| None)])
}

fn ran_so_far() -> Vec<(&'static str, Location)> {
    RAN.with_borrow(Clone::clone)
}

fn home() -> Location {
    Location::local(std::env::var_os("HOME").expect("the test's home"))
}

/// The middle of the element keyed `key`, once it is drawn.
fn middle(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let drawn = host.run_until(PATIENCE, |host| {
        let tree = host.tree();
        !all(&tree, &|node| node["key"] == key).is_empty()
    });
    assert!(drawn, "{key} is drawn");
    let elements = host.runtime().elements();
    let element = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = element.expect("the element is mounted").visible;
    (x + w / 2.0, y + h / 2.0)
}

fn press(host: &mut Offscreen, key: &str, button: PointerButton, mods: Mods) {
    let (x, y) = middle(host, key);
    host.runtime()
        .physical_input(PhysicalInput::Modifiers(mods));
    host.turn();
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    match button {
        PointerButton::Primary => runtime.pointer_down(x, y),
        PointerButton::Secondary => runtime.secondary_down(x, y),
        _ => runtime.button_down(x, y, button),
    }
    host.turn();
    let runtime = host.runtime();
    match button {
        PointerButton::Primary => runtime.pointer_up(x, y),
        PointerButton::Secondary => runtime.secondary_up(x, y),
        _ => runtime.button_up(x, y, button),
    }
    // A press is heard at its release, with the modifiers held then.
    host.turn();
    host.runtime()
        .physical_input(PhysicalInput::Modifiers(Mods::default()));
    idle(host);
}

fn opened(host: &mut Offscreen, folder: &Path) {
    let wanted = Location::local(folder);
    host.app().open(wanted.clone());
    let reached = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location() == Some(wanted.clone())
    });
    assert!(reached, "the window opens {}", folder.display());
    idle(host);
}

/// A tab opened at the end, shown, at `at`, while tab `first` stays at `stays`.
fn opened_in_a_tab(
    host: &mut Offscreen,
    count: usize,
    at: &Location,
    (first, stays): (TabKey, &Location),
) {
    let order = tabs(host.app());
    assert_eq!(order.len(), count, "a tab for each");
    assert_eq!(
        order.last(),
        Some(&shown_tab(host.app())),
        "the new tab, shown"
    );
    let reached = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location().as_ref() == Some(at)
    });
    assert!(reached, "the new tab opens {at:?}");
    let first = tab_browser(host.app(), first).expect("the first tab");
    assert_eq!(
        first.active_location().as_ref(),
        Some(stays),
        "the first tab stays"
    );
}

/// The key of the open menu's row named `name`.
fn menu_row(host: &mut Offscreen, name: &str) -> String {
    let row = |host: &Offscreen| {
        let tree = host.tree();
        let rows = all(&tree, &|node| {
            text(node, "role").starts_with("menu-item") && text(node, "name") == name
        });
        rows.first().map(|row| text(row, "key"))
    };
    assert!(
        host.run_until(PATIENCE, |host| row(host).is_some()),
        "the menu lists {name}"
    );
    row(host).expect("the row")
}

/// A middle click on a sidebar row opens its location in a new tab, as its menu's "Open in New
/// Tab" does, and its menu's other rows run their commands on the location the menu was opened
/// on.
#[test]
fn a_sidebar_rows_middle_click_and_menu_rows_run_on_its_location() {
    hosted_over(
        MODULE,
        "a_sidebar_rows_middle_click_and_menu_rows_run_on_its_location",
        with_the_commands,
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let first = (shown_tab(host.app()), &Location::local(folder.path()));
            press(
                &mut host,
                "sidebar-place-home",
                PointerButton::Middle,
                Mods::default(),
            );
            opened_in_a_tab(&mut host, 2, &home(), first);
            press(
                &mut host,
                "sidebar-place-home",
                PointerButton::Secondary,
                Mods::default(),
            );
            let row = menu_row(&mut host, "Open in New Tab");
            press(&mut host, &row, PointerButton::Primary, Mods::default());
            opened_in_a_tab(&mut host, 3, &home(), first);

            press(
                &mut host,
                "sidebar-place-home",
                PointerButton::Secondary,
                Mods::default(),
            );
            let row = menu_row(&mut host, "Customize…");
            press(&mut host, &row, PointerButton::Primary, Mods::default());
            assert_eq!(ran_so_far(), [("customize", home())]);
        },
    );
}

/// The key of the crumb that names `folder`.
fn crumb_of(host: &Offscreen, folder: &Path) -> String {
    let name = folder
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    let tree = host.tree();
    let crumbs = all(&tree, &|node| {
        text(node, "key").starts_with("location-crumb-") && text(node, "name") == name
    });
    text(crumbs.first().expect("the folder's crumb"), "key")
}

/// A crumb's middle click opens its folder in a new tab, and with Ctrl or Shift runs
/// `open-in-window`, as GTK's crumbs do; the tab it was pressed in stays where it is.
#[test]
fn a_crumbs_middle_click_runs_open_in_tab_or_with_a_modifier_in_window() {
    hosted_over(
        MODULE,
        "a_crumbs_middle_click_runs_open_in_tab_or_with_a_modifier_in_window",
        with_the_commands,
        |mut host| {
            let folder = fixture();
            let documents = folder.path().join("documents");
            opened(&mut host, &documents);
            let first = (shown_tab(host.app()), &Location::local(&documents));
            let root = Location::local(folder.path());
            let crumb = crumb_of(&host, folder.path());
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            for mods in [ctrl, shift] {
                press(&mut host, &crumb, PointerButton::Middle, mods);
            }
            assert_eq!(
                ran_so_far(),
                [
                    ("open-in-window", root.clone()),
                    ("open-in-window", root.clone())
                ]
            );
            assert_eq!(tabs(host.app()).len(), 1, "no tab for a new window");

            press(&mut host, &crumb, PointerButton::Middle, Mods::default());
            opened_in_a_tab(&mut host, 2, &root, first);
        },
    );
}
