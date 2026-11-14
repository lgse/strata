// SPDX-License-Identifier: MIT

//! What runs a command on a place, a location that is no row of the view: the sidebar's
//! menu rows and its middle click, and a crumb's middle click, each on the location it was
//! asked on (`On::Place`).

use std::{cell::RefCell, path::Path};

use rune::{Mods, PhysicalInput, PointerButton, Tx};
use strata_core::model::Location;

use super::super::{Action, Call, On, Outcome};
use super::routing::part;
use crate::{
    App,
    host::offscreen::Offscreen,
    slots::Entry,
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

fn in_tab(_: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    ran("open-in-tab", call)
}

fn in_window(_: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    ran("open-in-window", call)
}

fn customize(_: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    ran("customize", call)
}

const PLACES: &[Action<App>] = &[
    Action::new("open-in-tab", in_tab),
    Action::new("open-in-window", in_window),
    Action::new("customize", customize),
];

/// The registered parts, and one that registers the commands a place's routes name.
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

/// A middle click on a sidebar row runs `open-in-tab` on its location, and its menu's rows
/// run theirs on the location the menu was opened on.
#[test]
fn a_sidebar_rows_middle_click_and_menu_rows_run_on_its_location() {
    hosted_over(
        MODULE,
        "a_sidebar_rows_middle_click_and_menu_rows_run_on_its_location",
        with_the_commands,
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            press(
                &mut host,
                "sidebar-place-home",
                PointerButton::Middle,
                Mods::default(),
            );
            assert_eq!(ran_so_far(), [("open-in-tab", home())]);
            assert_eq!(
                host.app().browser().active_location(),
                Some(Location::local(folder.path())),
                "the middle click opens nothing in this tab"
            );
            for (row, command) in [
                ("Open in New Tab", "open-in-tab"),
                ("Customize…", "customize"),
            ] {
                press(
                    &mut host,
                    "sidebar-place-home",
                    PointerButton::Secondary,
                    Mods::default(),
                );
                let row = menu_row(&mut host, row);
                press(&mut host, &row, PointerButton::Primary, Mods::default());
                assert_eq!(ran_so_far().last(), Some(&(command, home())), "{row}");
            }
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

/// A crumb's middle click runs `open-in-tab` on its folder, and with Ctrl or Shift
/// `open-in-window`, as GTK's crumbs do; the tab stays where it is.
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
            let crumb = crumb_of(&host, folder.path());
            let root = Location::local(folder.path());
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            for mods in [Mods::default(), ctrl, shift] {
                press(&mut host, &crumb, PointerButton::Middle, mods);
            }
            assert_eq!(
                ran_so_far(),
                [
                    ("open-in-tab", root.clone()),
                    ("open-in-window", root.clone()),
                    ("open-in-window", root),
                ]
            );
            assert_eq!(
                host.app().browser().active_location(),
                Some(Location::local(&documents))
            );
        },
    );
}
