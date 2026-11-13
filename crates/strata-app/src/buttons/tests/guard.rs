// SPDX-License-Identifier: MIT

//! Every button the keyboard can reach takes Return, so the next button added cannot forget
//! it: each one comes through `buttons::on_return` or `buttons::taking_return`, sits in a
//! group whose own keys press its items, is a component's that takes Return itself
//! ([`IN_THEIR_COMPONENT`]), or is on [`EXCEPTIONS`].
//!
//! A group is told by its role alone, since an element's info does not say whether it is a
//! `focus_group()`: a `list` that is not one passes its buttons all the same.

use rune::{El, col, el, row};
use rune_kit::{Button, Kit, Popover};
use strata_core::model::Location;

use super::{MODULE, TAKING_RETURN, idle};
use crate::{
    App,
    error_dialog::{self, Error},
    host::offscreen::Offscreen,
    slots::{Entry, Frame, Place},
    test_support::{PATIENCE, fixture, hosted_over, idle_view, send_key, view},
};

/// Roles whose own keys move among and press their items, Return included.
const ITEM_GROUPS: [&str; 5] = ["list", "listbox", "menu", "tree", "grid"];

/// Buttons left to Rune's Return for now, each with why.
const EXCEPTIONS: [&str; 3] = [
    // The window's own buttons (C5's `window_buttons`): GTK's likely take no focus, so whether
    // these stop being Tab stops or take Return is the owner's (`owner.md`, A5).
    "header-minimize",
    "header-maximize",
    "header-close",
];

/// Buttons a component draws and presses on Return itself, by key prefix: C6's ancestor crumbs
/// (their own `on_key`) and its Copy path (`return_as_space` in its crumbs), which
/// `super::return_presses_copy_path_and_a_crumb` presses.
const IN_THEIR_COMPONENT: [&str; 2] = ["location-crumb-", "location-copy"];

/// C15's dialog panel, whose own keys turn Return into Space: the `dialog` keyed `{key}-dialog`
/// inside the layer keyed `{key}-layer` (`strata_components::dialog`'s shell). rune-kit gives
/// the same role to a popover's content and to its own dialog, which do not.
fn c15_dialog(elements: &[rune::ElementInfo], element: &rune::ElementInfo) -> bool {
    let key = element.key.as_deref().unwrap_or_default();
    let Some(base) = key.strip_suffix("-dialog") else {
        return false;
    };
    let layer = element
        .parent
        .and_then(|index| elements[index].key.as_deref());
    element.role == "dialog" && layer == Some(format!("{base}-layer").as_str())
}

/// Whether the group the button sits in presses it: the walk up stops at the first focusable
/// element, as Rune's `group()` does, since a control inside an item is no item.
fn left_to_its_group(elements: &[rune::ElementInfo], button: &rune::ElementInfo) -> bool {
    let mut at = button.parent;
    while let Some(index) = at {
        let element = &elements[index];
        if ITEM_GROUPS.contains(&element.role) || c15_dialog(elements, element) {
            return true;
        }
        if element.focusable {
            return false;
        }
        at = element.parent;
    }
    false
}

/// The focusable buttons of the window that neither take Return themselves nor are left to
/// their group, nor are named in [`EXCEPTIONS`]. The window is built again first, so the keys
/// recorded are those of the buttons drawn now, not of an earlier build under the same key
/// (Back without a history takes `on_return`).
fn deaf_to_return(host: &mut Offscreen) -> Vec<String> {
    TAKING_RETURN.lock().expect("the keys").clear();
    host.runtime().update(|_: &mut App, _| {});
    idle(host);
    let elements = host.runtime().elements();
    let taking = TAKING_RETURN.lock().expect("the keys");
    elements
        .iter()
        .filter(|element| element.role == "button" && element.focusable)
        .filter(|element| !element.inert && !element.exiting)
        .filter(|element| !left_to_its_group(&elements, element))
        .map(|element| element.key.clone().unwrap_or_else(|| element.path.clone()))
        .filter(|key| !taking.contains(key) && !EXCEPTIONS.contains(&key.as_str()))
        .filter(|key| {
            !IN_THEIR_COMPONENT
                .iter()
                .any(|prefix| key.starts_with(prefix))
        })
        .collect()
}

fn assert_all_take_return(host: &mut Offscreen) {
    let deaf = deaf_to_return(host);
    assert!(
        deaf.is_empty(),
        "buttons that Return does not press; give each `buttons::on_return`: {deaf:?}"
    );
}

/// Opens a folder and then one inside it, so Back has a history, and waits for the sidebar.
fn open_two_deep(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    for opened in [folder.path().to_owned(), folder.path().join("documents")] {
        let location = Location::local(&opened);
        host.app().open(location.clone());
        let shown = host.run_until(PATIENCE, |host| {
            host.app().browser().active_location().as_ref() == Some(&location)
        });
        assert!(shown, "{}", opened.display());
    }
    assert!(host.app().browser().can_go_back(), "Back has a history");
    let sidebar = host.run_until(PATIENCE, |host| {
        let tree = host.tree();
        !view::all(&tree, &|node| node["key"] == "sidebar-place-home").is_empty()
    });
    assert!(sidebar, "the sidebar is shown");
    idle(host);
    folder
}

fn every_button_takes_return(mode: &'static str, name: &str) {
    view::hosted_in(MODULE, mode, name, |mut host| {
        let _folder = open_two_deep(&mut host);
        assert_all_take_return(&mut host);
    });
}

#[test]
fn every_button_of_the_columns_view_takes_return() {
    every_button_takes_return(
        "columns",
        "guard::every_button_of_the_columns_view_takes_return",
    );
}

#[test]
fn every_button_of_the_icons_view_takes_return() {
    every_button_takes_return(
        "icons",
        "guard::every_button_of_the_icons_view_takes_return",
    );
}

#[test]
fn every_button_of_the_list_view_takes_return() {
    every_button_takes_return("list", "guard::every_button_of_the_list_view_takes_return");
}

#[test]
fn a_c15_dialogs_buttons_are_left_to_the_dialog() {
    let name = "guard::a_c15_dialogs_buttons_are_left_to_the_dialog";
    view::hosted_in(MODULE, "columns", name, |mut host| {
        let _folder = open_two_deep(&mut host);
        host.runtime().update(|app: &mut App, _| {
            error_dialog::show(app, Error::new("Title", "Message"));
        });
        idle(&mut host);
        let shown = host.runtime().elements().into_iter().any(|element| {
            element.key.as_deref() == Some("error-primary") && element.focusable && !element.inert
        });
        assert!(shown, "the dialog's button has the keyboard's reach");

        assert_all_take_return(&mut host);
    });
}

fn button(kit: &Kit, key: &str) -> El {
    Button::new(key.to_owned()).build(kit).key(key.to_owned())
}

/// Buttons in each kind of container, over a view that draws nothing else.
fn containers(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let kit = &app.look.kit;
    let button = |key| button(kit, key);
    let anchor = el().key("made-anchor").w(10.0).h(10.0);
    let popover = Popover::new("made-popover")
        .label("Popover")
        .trigger(anchor)
        .open(true)
        .child(button("made-popover-bare"))
        .child(crate::buttons::on_return(
            "made-popover-taking",
            Button::new("Taking".to_owned()).build(kit),
        ))
        .build(kit, frame.layer);
    let item = row()
        .key("made-item")
        .role("listitem")
        .focusable(true)
        .child(button("made-in-item"));
    let groups = col()
        .child(col().role("list").child(button("made-in-list")))
        .child(col().role("listbox").child(button("made-in-listbox")))
        .child(col().role("list").child(item));
    Some(col().key("made-containers").child(popover).child(groups))
}

fn made_view(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    Some(el().key("made-view").w(40.0).h(20.0).focusable(true))
}

fn made_parts() -> Vec<Entry<App>> {
    struct View;
    struct Containers;
    let entry = |name, kind, place| Entry {
        name,
        kind,
        place,
        listen: |_| None,
        publish: |_| {},
        caught_up: |_| {},
        poll: |_, _, _| false,
        switched: |_, _| {},
        commands: &[],
        keyboard: |_| None,
    };
    vec![
        entry(
            "made-view",
            std::any::TypeId::of::<View>,
            Place::View(idle_view("made-view", made_view, |_| true)),
        ),
        entry(
            "made-containers",
            std::any::TypeId::of::<Containers>,
            Place::Overlays(containers),
        ),
    ]
}

/// A popover is a `dialog` to assistive technology but does not press its buttons, and a
/// button inside a focusable item is no item of the list around it.
#[test]
fn a_popovers_button_and_a_button_inside_an_item_must_take_return_themselves() {
    let name = "guard::a_popovers_button_and_a_button_inside_an_item_must_take_return_themselves";
    hosted_over(MODULE, name, made_parts, |mut host| {
        idle(&mut host);
        let mut deaf = deaf_to_return(&mut host);
        deaf.sort();
        assert_eq!(deaf, ["made-in-item", "made-popover-bare"]);
    });
}

/// The Icons header's "Thumbnail size" popover, its two actions shown: a popover's buttons are
/// checked like the rest.
#[test]
fn every_button_of_the_thumbnail_size_popover_takes_return() {
    let name = "guard::every_button_of_the_thumbnail_size_popover_takes_return";
    view::hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = open_two_deep(&mut host);
        let focus = |key: &'static str| move |_: &mut App, tx: &mut rune::Tx| tx.focus(key, true);
        let drawn = |host: &Offscreen, key: &str| {
            let tree = host.tree();
            !view::all(&tree, &|node| node["key"] == key).is_empty()
        };
        let key = |host: &mut Offscreen, key: rune::Key| {
            let mods = rune::Mods::default();
            send_key(host, rune::KeyEvent { key, mods });
        };
        host.runtime().update(focus("icons-thumbnail-size"));
        idle(&mut host);
        key(&mut host, rune::Key::Enter);
        let opened = host.run_until(PATIENCE, |host| drawn(host, "icons-thumbnail-slider"));
        assert!(opened, "the popover");
        host.runtime().update(focus("icons-thumbnail-slider"));
        idle(&mut host);
        key(&mut host, rune::Key::Right);
        let offered = host.run_until(PATIENCE, |host| drawn(host, "icons-thumbnail-make-default"));
        assert!(offered, "the popover's actions");

        assert_all_take_return(&mut host);
    });
}
