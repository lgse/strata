// SPDX-License-Identifier: MIT

//! The registry over made-up parts: what is refused, who is in a slot, which view is shown,
//! and the order the parts hear the browser in.

use std::{any::TypeId, rc::Rc};

use rune::{Env, Runtime, el};
use strata_core::model::ViewPreferences;
use strata_desktop::settings::Preferences;
use strata_fs::{
    adapters::LocalFileSource,
    app::{Browser, BrowserEvent},
};

use super::{Entry, Inbox, Listener, Registry, Setup, Slot, registry::Fault};
use crate::columns::Ids;

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
    Editor = "editor",
    Menu = "menu",
}

fn named(event: &BrowserEvent) -> &'static str {
    match event {
        BrowserEvent::Reset => "reset",
        BrowserEvent::HiddenToggled { .. } => "hidden",
        _ => "another event",
    }
}

/// A listener that queues one line for each event: who heard it, and which it was.
fn listen<P: Made>(_: &Setup<'_>) -> Option<Listener<Log>> {
    Some(Box::new(|event, _, heard| {
        let event = named(event);
        heard.then(move |log: &mut Log, _| {
            log.push(format!("{} heard {event}", P::NAME));
            true
        });
    }))
}

/// Part `P` in `slot`, with hooks that do nothing and a listener.
fn part<P: Made>(slot: Slot) -> Entry<Log> {
    Entry {
        name: P::NAME,
        slot,
        focus: None,
        kind: TypeId::of::<P>,
        shows: |mode| mode == P::NAME,
        listen: listen::<P>,
        build: |_, _| None,
        wrap: |_, files, _| files,
        publish: |_| {},
        caught_up: |_| {},
        poll: |_, _, _| false,
        pointer_at: |_, _| {},
        holds_keyboard: |_| false,
    }
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
        ..part::<Columns>(Slot::View)
    };
    let cases = [
        (
            vec![part::<Columns>(Slot::View), part::<Columns>(Slot::View)],
            Err(Fault::NamedTwice("columns")),
        ),
        (
            vec![part::<Columns>(Slot::View), renamed],
            Err(Fault::SamePart("columns", "strip")),
        ),
        (
            vec![part::<Sidebar>(Slot::Sidebar), part::<Menu>(Slot::Sidebar)],
            Err(Fault::Crowded(Slot::Sidebar, "sidebar", "menu")),
        ),
        (
            vec![
                part::<Columns>(Slot::View),
                part::<Icons>(Slot::View),
                part::<Editor>(Slot::Overlays),
                part::<Menu>(Slot::Overlays),
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
fn a_slot_nothing_is_registered_in_has_no_part_and_no_focus() {
    let columns = Entry {
        focus: Some("strip"),
        ..part::<Columns>(Slot::View)
    };
    let registry = Registry::new(table(vec![columns]), "columns");

    assert_eq!(registry.focus(Slot::View), Some("strip"));
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
        assert_eq!(registry.focus(empty), None, "{empty:?}");
    }
}

#[test]
fn the_saved_mode_names_the_view_that_is_shown_and_the_first_stands_in() {
    let entries = table(vec![
        part::<Editor>(Slot::Overlays),
        part::<Columns>(Slot::View),
        part::<Icons>(Slot::View),
        part::<Menu>(Slot::Overlays),
    ]);
    let cases = [
        ("columns", ["editor", "columns", "menu"]),
        ("icons", ["editor", "icons", "menu"]),
        // A mode no registered view answers to, as "list" is until there is a List view.
        ("list", ["editor", "columns", "menu"]),
    ];

    for (mode, active) in cases {
        let registry = Registry::new(entries, mode);
        assert_eq!(names(registry.active()), active, "{mode}");
        assert_eq!(names(registry.of(Slot::View)), [active[1]], "{mode}");
    }
}

#[test]
fn a_window_with_no_view_registered_shows_none() {
    let registry = Registry::new(table(vec![part::<Menu>(Slot::Overlays)]), "columns");

    assert_eq!(names(registry.of(Slot::View)), [""; 0]);
    assert_eq!(names(registry.active()), ["menu"]);
}

#[test]
fn parts_hear_each_event_in_the_order_they_are_registered_and_a_hidden_view_hears_none() {
    let entries = table(vec![
        part::<Editor>(Slot::Overlays),
        part::<Icons>(Slot::View),
        part::<Columns>(Slot::View),
        part::<Menu>(Slot::Overlays),
    ]);
    let registry = Registry::new(entries, "columns");
    let (settings, ids) = (Preferences::default(), Ids::default());
    let listeners = registry.listeners(&Setup {
        settings: &settings,
        ids: &ids,
    });
    let browser = Browser::with_preferences(Rc::new(LocalFileSource), ViewPreferences::default());
    let inbox = Inbox::observing(&browser, Vec::new());

    inbox.hear(&BrowserEvent::Reset, &browser, &listeners);
    inbox.hear(
        &BrowserEvent::HiddenToggled { show_hidden: true },
        &browser,
        &listeners,
    );
    inbox.push(|log: &mut Log, _| {
        log.push("pushed".to_owned());
        false
    });

    assert_eq!(inbox.heard(), 2);
    let mut runtime = Runtime::new(Log::new(), |_, _| el(), (64.0, 64.0), Env::new(true, 1.0));
    let mut asked_for_a_view = Vec::new();
    runtime.update(|log, tx| {
        while let Some(update) = inbox.pop() {
            asked_for_a_view.push(update(log, tx));
        }
    });
    assert_eq!(
        runtime.state,
        [
            "editor heard reset",
            "columns heard reset",
            "menu heard reset",
            "editor heard hidden",
            "columns heard hidden",
            "menu heard hidden",
            "pushed",
        ]
    );
    assert_eq!(
        asked_for_a_view,
        [true, true, true, true, true, true, false]
    );
    assert!(inbox.is_empty());
}
