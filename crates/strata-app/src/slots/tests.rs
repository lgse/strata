// SPDX-License-Identifier: MIT

//! The registry over made-up parts: what is refused, who is in a slot, which view is shown,
//! and the order the parts hear the browser in.

use std::{any::TypeId, rc::Rc};

use rune::{El, Env, Runtime, el};
use strata_components::{contract::EntryId, rename::Polled};
use strata_core::model::ViewPreferences;
use strata_desktop::settings::Preferences;
use strata_fs::{
    adapters::LocalFileSource,
    app::{Browser, BrowserEvent},
};

#[cfg(feature = "qa")]
use super::Probe;
use super::{
    Editing, Entry, Frame, Ids, Inbox, Listener, Place, Registry, Setup, Slot, View,
    registry::Fault,
};

/// What the made-up parts did, in order.
type Log = Vec<String>;

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
    Columns = "columns",
    Icons = "icons",
    Sidebar = "sidebar",
    Drawer = "drawer",
    Editor = "editor",
    Menu = "menu",
}

/// A listener that queues one line for each change of the hidden-files choice: who heard it,
/// and what the choice was.
fn listen<P: Made>(_: &Setup<'_>) -> Option<Listener<Log>> {
    Some(Box::new(|event, _, heard| {
        if let BrowserEvent::HiddenToggled { show_hidden } = event {
            let shown = *show_hidden;
            heard.then(move |log: &mut Log, _| {
                log.push(format!("{} heard {shown}", P::NAME));
                true
            });
        }
    }))
}

/// The element of a made-up part: none.
fn nothing(_: &Log, _: &mut Frame<'_, Log>) -> Option<El> {
    None
}

/// Part `P` at `place`, with hooks that do nothing and a listener.
fn part<P: Made>(place: Place<Log>) -> Entry<Log> {
    Entry {
        name: P::NAME,
        kind: TypeId::of::<P>,
        place,
        listen: listen::<P>,
        publish: |_| {},
        caught_up: |_| {},
        poll: |_, _, _| false,
        holds_keyboard: |_| false,
    }
}

/// A view that answers to its own name as the saved mode, and takes the keyboard there.
fn view<P: Made>() -> Entry<Log> {
    part::<P>(Place::View(View {
        build: nothing,
        shows: |mode| mode == P::NAME,
        focus: P::NAME,
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

fn overlay<P: Made>() -> Entry<Log> {
    part::<P>(Place::Overlays(nothing))
}

fn names(parts: impl Iterator<Item = &'static Entry<Log>>) -> Vec<&'static str> {
    parts.map(|part| part.name).collect()
}

/// `entries`, for as long as the test process lives: a registry borrows its table.
fn table(entries: Vec<Entry<Log>>) -> &'static [Entry<Log>] {
    entries.leak()
}

#[test]
fn a_part_registered_twice_is_refused_and_so_is_a_second_part_in_a_slot_for_one() {
    let renamed = Entry {
        name: "strip",
        ..view::<Columns>()
    };
    let cases = [
        (
            vec![view::<Columns>(), view::<Columns>()],
            Err(Fault::NamedTwice("columns")),
        ),
        (
            vec![view::<Columns>(), renamed],
            Err(Fault::SamePart("columns", "strip")),
        ),
        (
            vec![
                part::<Sidebar>(Place::Sidebar(nothing)),
                part::<Menu>(Place::Sidebar(nothing)),
            ],
            Err(Fault::Crowded(Slot::Sidebar, "sidebar", "menu")),
        ),
        (
            vec![
                view::<Columns>(),
                view::<Icons>(),
                overlay::<Editor>(),
                overlay::<Menu>(),
            ],
            Ok(()),
        ),
    ];

    for (entries, expected) in cases {
        let entries = table(entries);
        let registered = names(entries.iter());
        assert_eq!(Registry::check(entries), expected, "{registered:?}");
    }
}

#[test]
fn the_parts_this_crate_registers_can_be_a_window() {
    assert_eq!(Registry::check(crate::PARTS), Ok(()));
}

#[test]
fn a_slot_nothing_is_registered_in_has_no_part() {
    let registry = Registry::new(table(vec![view::<Columns>()]), "columns");

    assert_eq!(names(registry.of(Slot::View)), ["columns"]);
    for empty in [
        Slot::Tabs,
        Slot::Location,
        Slot::Sidebar,
        Slot::Drawer,
        Slot::Footer,
        Slot::Overlays,
        Slot::Dialogs,
    ] {
        assert_eq!(names(registry.of(empty)), [""; 0], "{empty:?}");
    }
    assert!(registry.drawer().is_none());
}

#[test]
fn a_part_builds_its_element_in_every_slot_but_the_drawer_which_wraps_the_views() {
    let places = [
        (Place::Tabs(nothing), Slot::Tabs, true),
        (Place::Location(nothing), Slot::Location, true),
        (Place::Sidebar(nothing), Slot::Sidebar, true),
        (view::<Columns>().place, Slot::View, true),
        (Place::Drawer(|_, files, _| files), Slot::Drawer, false),
        (Place::Footer(nothing), Slot::Footer, true),
        (Place::Overlays(nothing), Slot::Overlays, true),
        (Place::Dialogs(nothing), Slot::Dialogs, true),
    ];

    for (place, slot, builds) in places {
        assert_eq!(place.slot(), slot);
        assert_eq!(place.build().is_some(), builds, "{slot:?}");
        let registry = Registry::new(table(vec![part::<Drawer>(place)]), "columns");
        assert_eq!(names(registry.of(slot)), ["drawer"], "{slot:?}");
        assert_eq!(registry.drawer().is_some(), !builds, "{slot:?}");
    }
}

#[test]
fn the_saved_mode_names_the_view_that_is_shown_and_the_first_stands_in() {
    let entries = table(vec![
        overlay::<Editor>(),
        view::<Columns>(),
        view::<Icons>(),
        overlay::<Menu>(),
    ]);
    let cases = [
        ("columns", ["editor", "columns", "menu"]),
        ("icons", ["editor", "icons", "menu"]),
        // A mode no registered view answers to, as "list" is until there is a List view.
        ("list", ["editor", "columns", "menu"]),
    ];

    for (mode, active) in cases {
        let registry = Registry::new(entries, mode);
        let shown = registry.shown().map(|view| view.focus);
        assert_eq!(names(registry.active()), active, "{mode}");
        assert_eq!(names(registry.of(Slot::View)), [active[1]], "{mode}");
        assert_eq!(shown, Some(active[1]), "{mode}");
    }
}

#[test]
fn a_window_with_no_view_registered_shows_none() {
    let registry = Registry::new(table(vec![overlay::<Menu>()]), "columns");

    assert_eq!(names(registry.of(Slot::View)), [""; 0]);
    assert_eq!(names(registry.active()), ["menu"]);
    assert!(registry.shown().is_none());
}

/// The registry's listeners on a new browser's observer, as the shell puts them, with the
/// queue they fill.
fn observed(entries: &'static [Entry<Log>]) -> (Rc<Browser>, Inbox<Log>) {
    let registry = Registry::new(entries, "columns");
    let (settings, ids, inbox) = (Preferences::default(), Ids::default(), Inbox::new());
    let setup = Setup {
        settings: &settings,
        ids: &ids,
        // The made-up parts are over a log, and the handle is the window's.
        later: &Inbox::new().later(),
    };
    let browser = Browser::with_preferences(Rc::new(LocalFileSource), ViewPreferences::default());
    inbox.observe(&browser, registry.listeners(&setup));
    (browser, inbox)
}

/// Applies everything `inbox` holds to a new log. Returns the log, and what each update said
/// of whether the view has to run.
fn applied(inbox: &Inbox<Log>) -> (Log, Vec<bool>) {
    let mut runtime = Runtime::new(Log::new(), |_, _| el(), (64.0, 64.0), Env::new(true, 1.0));
    let mut asks_for_a_view = Vec::new();
    runtime.update(|log, tx| {
        while let Some(update) = inbox.pop() {
            asks_for_a_view.push(update(log, tx));
        }
    });
    (runtime.state, asks_for_a_view)
}

/// What the parts of `entries` logged after the browser changed its hidden-files choice
/// twice.
fn heard_by(entries: &'static [Entry<Log>]) -> Log {
    let (browser, inbox) = observed(entries);

    browser.toggle_hidden();
    browser.toggle_hidden();

    assert_eq!(inbox.heard(), 2);
    applied(&inbox).0
}

#[test]
fn parts_hear_each_event_in_the_order_they_are_registered_and_a_hidden_view_hears_none() {
    let one_way = table(vec![
        overlay::<Editor>(),
        view::<Icons>(),
        view::<Columns>(),
        overlay::<Menu>(),
    ]);
    let the_other = table(vec![
        overlay::<Menu>(),
        view::<Columns>(),
        view::<Icons>(),
        overlay::<Editor>(),
    ]);

    assert_eq!(
        heard_by(one_way),
        [
            "editor heard true",
            "columns heard true",
            "menu heard true",
            "editor heard false",
            "columns heard false",
            "menu heard false",
        ]
    );
    assert_eq!(
        heard_by(the_other),
        [
            "menu heard true",
            "columns heard true",
            "editor heard true",
            "menu heard false",
            "columns heard false",
            "editor heard false",
        ]
    );
}

#[test]
fn what_a_service_queues_waits_its_turn_among_the_browsers_events() {
    let (browser, inbox) = observed(table(vec![overlay::<Menu>()]));
    let later = inbox.later();

    browser.toggle_hidden();
    later.then(|log: &mut Log, _| {
        log.push("the service said so".to_owned());
        false
    });
    browser.toggle_hidden();

    let (log, asks_for_a_view) = applied(&inbox);
    assert_eq!(
        log,
        ["menu heard true", "the service said so", "menu heard false"]
    );
    assert_eq!(asks_for_a_view, [true, false, true]);
    assert!(inbox.is_empty());
}
