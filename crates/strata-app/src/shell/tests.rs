// SPDX-License-Identifier: MIT

//! The shell over a made-up table of parts, one in every slot: where each lands in the tree,
//! who gets the keyboard back, and the order the shell calls them in.

use std::cell::{Cell, RefCell};

use rune::{El, Key, KeyEvent, Mods, PhysicalInput, Tx, el};
use serde_json::Value;
use strata_components::{contract::EntryId, rename::Polled};
use strata_fs::app::BrowserEvent;

#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App,
    host::offscreen::Offscreen,
    slots::{Editing, Entry, Frame, Later, Listener, Place, Setup, View},
};

const MODULE: &str = "shell::tests";

thread_local! {
    /// What the made-up parts were asked, in order.
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Whether the made-up field has the keyboard.
    static TYPING: Cell<bool> = const { Cell::new(false) };
    /// The way into the window's queue, as a part's state would keep it.
    static LATER: RefCell<Option<Later<App>>> = const { RefCell::new(None) };
}

fn note(line: impl Into<String>) {
    LOG.with_borrow_mut(|log| log.push(line.into()));
}

fn noted() -> Vec<String> {
    LOG.with_borrow_mut(std::mem::take)
}

/// A made-up part: its type tells two of them apart, as a real part's state does.
trait Made: 'static {
    const NAME: &'static str;
}

macro_rules! made {
    ($($part:ident = $name:literal,)*) => {
        $(
            struct $part;

            impl Made for $part {
                const NAME: &'static str = $name;
            }
        )*
    };
}

made! {
    Tabs = "tabs",
    Location = "location",
    Sidebar = "sidebar",
    Files = "files",
    Drawer = "drawer",
    Status = "status",
    Hints = "hints",
    Menu = "menu",
    Dialog = "dialog",
}

/// The element a made-up part leaves in the tree, keyed with its name.
fn placed<P: Made>(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    Some(el().key(format!("made-{}", P::NAME)))
}

/// As [`placed`], with a surface in the layer.
fn floated<P: Made>(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    frame
        .layer
        .push(el().key(format!("made-{}-surface", P::NAME)));
    placed::<P>(app, frame)
}

fn wrapped<P: Made>(_: &App, files: El, _: &mut Frame<'_, App>) -> El {
    el().key(format!("made-{}", P::NAME)).child(files)
}

/// Keeps the way into the queue, and queues a line for each change of the hidden-files
/// choice.
fn listen<P: Made>(setup: &Setup<'_>) -> Option<Listener<App>> {
    LATER.set(Some(setup.later.clone()));
    Some(Box::new(|event, _, heard| {
        if matches!(event, BrowserEvent::HiddenToggled { .. }) {
            heard.then(|_: &mut App, _: &mut Tx| {
                note(format!("{} applies", P::NAME));
                true
            });
        }
    }))
}

/// Part `P` at `place`, with hooks that say they were called.
fn part<P: Made>(place: Place<App>) -> Entry<App> {
    Entry {
        name: P::NAME,
        kind: std::any::TypeId::of::<P>,
        place,
        listen: listen::<P>,
        publish: |_| note(format!("{} publishes", P::NAME)),
        caught_up: |_| note(format!("{} has caught up", P::NAME)),
        poll: |_, _, _| {
            note(format!("{} polls", P::NAME));
            false
        },
        holds_keyboard: |_| TYPING.get(),
    }
}

fn files() -> Entry<App> {
    part::<Files>(Place::View(View {
        build: placed::<Files>,
        shows: |_| true,
        focus: "made-files",
        pointer_at: None,
        editing: Editing {
            id_of: |_, _| EntryId::new(0),
            position: |_, _| None,
            neighbour: |_, _, _| None,
            begin: |_, _, _| None,
            tick: |_| Polled::default(),
            hold: |_, _| {},
            select_only: |_, _, _| {},
            follow_now: |_| {},
        },
        chosen: |_, _, _| {},
        #[cfg(feature = "qa")]
        probe: Probe {
            listed: |_| true,
            lists: |_| Vec::new(),
        },
    }))
}

/// A part in every slot, and two where a slot takes several, registered against the order
/// of the slots so that the order seen is the shell's and not the table's.
fn every_slot() -> Vec<Entry<App>> {
    vec![
        part::<Dialog>(Place::Dialogs(floated::<Dialog>)),
        part::<Menu>(Place::Overlays(floated::<Menu>)),
        part::<Status>(Place::Footer(placed::<Status>)),
        part::<Hints>(Place::Footer(placed::<Hints>)),
        part::<Drawer>(Place::Drawer(wrapped::<Drawer>)),
        files(),
        part::<Sidebar>(Place::Sidebar(placed::<Sidebar>)),
        part::<Location>(Place::Location(placed::<Location>)),
        part::<Tabs>(Place::Tabs(placed::<Tabs>)),
    ]
}

fn two_parts() -> Vec<Entry<App>> {
    vec![part::<Menu>(Place::Overlays(placed::<Menu>)), files()]
}

fn hosted(name: &str, parts: fn() -> Vec<Entry<App>>, run: impl FnOnce(Offscreen)) {
    crate::test_support::hosted_over(MODULE, name, parts, run);
}

/// The made-up parts' keys as the accessibility tree has them, in its order, each with the
/// made-up keys it is inside of.
fn made(node: &Value, inside: &[String], found: &mut Vec<String>) {
    let key = node["key"].as_str().filter(|key| key.starts_with("made-"));
    let mut inside = inside.to_vec();
    if let Some(key) = key {
        inside.push(key.to_owned());
        found.push(inside.join(" > "));
    }
    for child in node["children"].as_array().into_iter().flatten() {
        made(child, &inside, found);
    }
}

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    let runtime = host.runtime();
    runtime.physical_input(PhysicalInput::Modifiers(mods));
    runtime.key(KeyEvent { key, mods });
    runtime.physical_input(PhysicalInput::Modifiers(Mods::default()));
    while host.turn() {}
}

#[test]
fn every_slots_part_is_in_the_tree_in_the_order_of_the_slots() {
    hosted(
        "every_slots_part_is_in_the_tree_in_the_order_of_the_slots",
        every_slot,
        |mut host| {
            while host.turn() {}

            let mut found = Vec::new();
            made(&host.tree(), &[], &mut found);
            assert_eq!(
                found,
                [
                    "made-tabs",
                    "made-location",
                    "made-sidebar",
                    // The drawer was handed the view's element, and put it in its own.
                    "made-drawer",
                    "made-drawer > made-files",
                    "made-status",
                    "made-hints",
                    "made-menu",
                    "made-dialog",
                    // The layer, over the window: the overlay's surface under the dialog's.
                    "made-menu-surface",
                    "made-dialog-surface",
                ]
            );
        },
    );
}

#[test]
fn the_keyboard_goes_back_to_the_shown_view_and_the_windows_keys_wait_for_a_part_that_holds_it() {
    hosted(
        "the_keyboard_goes_back_to_the_shown_view_and_the_windows_keys_wait_for_a_part_that_holds_it",
        two_parts,
        |mut host| {
            let hidden_shown = |host: &Offscreen| host.app().browser().preferences().show_hidden;
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            assert_eq!(crate::shell::view_focus(host.app()), Some("made-files"));
            let before = hidden_shown(&host);

            TYPING.set(true);
            press(&mut host, Key::Text("h".to_owned()), ctrl);
            assert_eq!(hidden_shown(&host), before, "the key waited");

            TYPING.set(false);
            press(&mut host, Key::Text("h".to_owned()), ctrl);
            assert_eq!(hidden_shown(&host), !before, "the key was the window's");
        },
    );
}

#[test]
fn the_shell_calls_the_parts_in_the_order_they_are_registered() {
    hosted(
        "the_shell_calls_the_parts_in_the_order_they_are_registered",
        two_parts,
        |mut host| {
            while host.turn() {}
            noted();

            host.app().browser().toggle_hidden();
            host.turn();

            let log = noted();
            let first_poll = log.iter().position(|line| line == "files polls");
            let one_poll = &log[..=first_poll.expect("a poll")];
            assert_eq!(
                one_poll,
                [
                    // Before GLib dispatches.
                    "menu publishes",
                    "files publishes",
                    // The poll: what the parts changed, the queue, what that left to say.
                    "menu publishes",
                    "files publishes",
                    "menu applies",
                    "files applies",
                    "menu has caught up",
                    "files has caught up",
                    "menu publishes",
                    "files publishes",
                    "menu polls",
                    "files polls",
                ]
            );
        },
    );
}

#[test]
fn what_a_services_callback_queues_reaches_the_window_within_one_turn() {
    hosted(
        "what_a_services_callback_queues_reaches_the_window_within_one_turn",
        two_parts,
        |mut host| {
            while host.turn() {}
            noted();
            let later = LATER.take().expect("the part kept the way in");
            // A service calls back inside GLib's dispatch, as the browser's observer is.
            glib::idle_add_local_once(move || {
                later.then(|_: &mut App, tx: &mut Tx| {
                    note("the service's update ran");
                    tx.set_title("a volume arrived");
                    true
                });
            });
            assert!(!host.app().has_queued_changes());

            host.turn();

            let log = noted();
            assert!(
                log.contains(&"the service's update ran".to_owned()),
                "{log:?}"
            );
            assert!(!host.app().has_queued_changes());
            // Its turn was in the poll's queue, before the parts were told they had caught up.
            let ran = log.iter().position(|line| line.starts_with("the service"));
            let caught_up = log.iter().position(|line| line == "menu has caught up");
            assert!(ran < caught_up, "{log:?}");
        },
    );
}
