// SPDX-License-Identifier: MIT

//! The shell over a made-up table of parts, one in every slot: where each lands in the tree,
//! where the keyboard goes when it leaves the view and comes back, the order the shell calls
//! the parts in, and how what a service says reaches the window.

use std::cell::{Cell, RefCell};

use rune::{El, Key, KeyEvent, Mods, Tx, el};
use serde_json::Value;
use strata_fs::app::BrowserEvent;

use crate::{
    App,
    commands::Holds,
    host::offscreen::Offscreen,
    shell,
    slots::{Entry, Frame, Listener, Place, Setup, Sidebar},
    test_support::{idle_view, send_key},
};

const MODULE: &str = "shell::tests";

thread_local! {
    /// What the made-up parts were asked, in order.
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Whether the made-up field has the keyboard.
    static TYPING: Cell<bool> = const { Cell::new(false) };
    /// A made-up part's next poll asks a service that answers at once.
    static ASKING: Cell<bool> = const { Cell::new(false) };
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
    Rail = "sidebar",
    Files = "files",
    Drawer = "drawer",
    Status = "status",
    Hints = "hints",
    Menu = "menu",
    Dialog = "dialog",
}

/// The element a made-up part leaves in the tree, keyed with its name. It has a size, so it
/// can take the keyboard.
fn placed<P: Made>(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    let key = format!("made-{}", P::NAME);
    Some(el().key(key).w(40.0).h(20.0).focusable(true))
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

/// Queues a line for each change of the hidden-files choice.
fn listen<P: Made>(_: &Setup<'_>) -> Option<Listener<App>> {
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
        poll: |app, _, _| {
            note(format!("{} polls", P::NAME));
            if ASKING.take() {
                app.later().then(|_: &mut App, _: &mut Tx| {
                    note("the answer was applied");
                    true
                });
            }
            false
        },
        commands: &[],
        keyboard: |_| TYPING.get().then_some(Holds::Edit),
    }
}

fn files() -> Entry<App> {
    let view = idle_view("made-files", placed::<Files>, |_| true);
    part::<Files>(Place::View(view))
}

fn sidebar() -> Entry<App> {
    part::<Rail>(Place::Sidebar(Sidebar {
        build: placed::<Rail>,
        focus: "made-sidebar",
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
        sidebar(),
        part::<Location>(Place::Location(placed::<Location>)),
        part::<Tabs>(Place::Tabs(placed::<Tabs>)),
    ]
}

fn two_parts() -> Vec<Entry<App>> {
    vec![part::<Menu>(Place::Overlays(placed::<Menu>)), files()]
}

fn a_view_and_a_sidebar() -> Vec<Entry<App>> {
    vec![files(), sidebar()]
}

fn hosted(name: &str, parts: fn() -> Vec<Entry<App>>, run: impl FnOnce(Offscreen)) {
    crate::test_support::hosted_over(MODULE, name, parts, run);
}

/// The shell's own two regions, as it keys them.
const REGIONS: [&str; 2] = ["header", "body"];

/// The made-up parts' keys and the shell's regions as the accessibility tree has them, in its
/// order, each with those of them it is inside of.
fn made(node: &Value, inside: &[String], found: &mut Vec<String>) {
    let ours = |key: &&str| key.starts_with("made-") || REGIONS.contains(key);
    let key = node["key"].as_str().filter(ours);
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
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

/// The key of the element that has the keyboard, as the accessibility tree says.
fn focused(node: &Value) -> Option<String> {
    let has = |state: &Value| state == "focused";
    if node["states"]
        .as_array()
        .is_some_and(|states| states.iter().any(has))
    {
        return node["key"].as_str().map(str::to_owned);
    }
    let children = node["children"].as_array().into_iter().flatten();
    children.filter_map(focused).next()
}

/// Runs `act` on the state as a handler would, and lets the window settle.
fn act<T>(host: &mut Offscreen, act: impl FnOnce(&mut App, &mut Tx) -> T) -> T {
    let mut said = None;
    host.runtime().update(|app, tx| said = Some(act(app, tx)));
    while host.turn() {}
    said.expect("the update ran")
}

#[test]
fn every_slots_part_is_in_the_tree_where_the_slots_table_puts_it() {
    hosted(
        "every_slots_part_is_in_the_tree_where_the_slots_table_puts_it",
        every_slot,
        |mut host| {
            while host.turn() {}

            let mut found = Vec::new();
            made(&host.tree(), &[], &mut found);
            assert_eq!(
                found,
                [
                    "made-tabs",
                    "header",
                    "header > made-location",
                    "body",
                    "body > made-sidebar",
                    // The drawer was handed the view's element, and put it in its own.
                    "body > made-drawer",
                    "body > made-drawer > made-files",
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
fn the_keyboard_leaves_the_view_for_the_sidebar_and_comes_back_to_the_view() {
    hosted(
        "the_keyboard_leaves_the_view_for_the_sidebar_and_comes_back_to_the_view",
        a_view_and_a_sidebar,
        |mut host| {
            let back = |app: &mut App, tx: &mut Tx| {
                let view = shell::view_focus(app).expect("a view is shown");
                tx.focus(view, false);
            };
            act(&mut host, back);
            assert_eq!(focused(&host.tree()).as_deref(), Some("made-files"));

            let taken = act(&mut host, |app, tx| shell::leave_view_at_start(app, tx));

            assert!(taken, "the sidebar takes the keyboard");
            assert_eq!(focused(&host.tree()).as_deref(), Some("made-sidebar"));

            act(&mut host, back);
            assert_eq!(focused(&host.tree()).as_deref(), Some("made-files"));
        },
    );
}

#[test]
fn the_keyboard_stays_in_the_view_when_no_sidebar_is_registered() {
    hosted(
        "the_keyboard_stays_in_the_view_when_no_sidebar_is_registered",
        two_parts,
        |mut host| {
            act(&mut host, |app, tx| {
                let view = shell::view_focus(app).expect("a view is shown");
                tx.focus(view, false);
            });

            let taken = act(&mut host, |app, tx| shell::leave_view_at_start(app, tx));

            assert!(!taken);
            assert_eq!(focused(&host.tree()).as_deref(), Some("made-files"));
        },
    );
}

#[test]
fn the_windows_keys_wait_for_a_part_that_holds_the_keyboard() {
    hosted(
        "the_windows_keys_wait_for_a_part_that_holds_the_keyboard",
        two_parts,
        |mut host| {
            let hidden_shown = |host: &Offscreen| host.app().browser().preferences().show_hidden;
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
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

/// The offscreen host dispatches GLib itself and then applies the queue. The windowed host is
/// not run here: it applies the same queue after every dispatch (`host::windowed::pump`), and
/// its GLib loop reaches Rune's with `AppHandle::update_if`.
#[test]
fn what_a_services_callback_queues_reaches_the_window_within_one_turn() {
    hosted(
        "what_a_services_callback_queues_reaches_the_window_within_one_turn",
        two_parts,
        |mut host| {
            while host.turn() {}
            noted();
            // A part keeps the handle `Setup` gave it. A service calls back inside GLib's
            // dispatch, as the browser's observer is called.
            let later = host.app().later();
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
            let ran = log.iter().position(|line| line.starts_with("the service"));
            let caught_up = log.iter().position(|line| line == "menu has caught up");
            assert!(ran.is_some(), "{log:?}");
            // Its turn was in the poll's queue, before the parts were told they had caught up.
            assert!(ran < caught_up, "{log:?}");
            assert!(!host.app().has_queued_changes());
        },
    );
}

#[test]
fn what_a_service_answers_at_once_from_a_parts_poll_is_applied_in_that_poll() {
    hosted(
        "what_a_service_answers_at_once_from_a_parts_poll_is_applied_in_that_poll",
        two_parts,
        |mut host| {
            while host.turn() {}
            noted();

            ASKING.set(true);
            host.turn();

            let log = noted();
            assert!(
                log.contains(&"the answer was applied".to_owned()),
                "{log:?}"
            );
            assert!(!host.app().has_queued_changes());
        },
    );
}

/// GTK logs a saved keymap's unusable entries once, as its preference manager first builds
/// the tables; here the process's first window does, and no later one.
#[test]
fn only_the_first_window_logs_the_saved_keymap() {
    let test = "shell::tests::only_the_first_window_logs_the_saved_keymap";
    crate::test_support::in_own_process(test, &[], || {
        assert_eq!(
            super::first_window_reports(),
            Some(strata_desktop::settings::settings_path())
        );
        assert_eq!(super::first_window_reports(), None);
    });
}
