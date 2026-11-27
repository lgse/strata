// SPDX-License-Identifier: MIT

use std::{any::TypeId, rc::Rc};

use rune::{El, Env, Runtime, el};
use strata_core::model::ViewPreferences;
use strata_desktop::settings::Preferences;
use strata_fs::{
    adapters::LocalFileSource,
    app::{Browser, BrowserEvent},
};

use super::{
    Entry, Frame, Ids, Inbox, Listener, Place, Registry, Setup, ShownView, Sidebar, Slot, Strip,
    Worn, registry::Fault,
};
use crate::{
    commands::{Action, Call, Outcome},
    test_support::idle_view,
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
    Rail = "sidebar",
    Drawer = "drawer",
    Editor = "editor",
    Menu = "menu",
}

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
        customized: |_| {},
        poll: |_, _, _| false,
        switched: |_, _| {},
        tab_hidden: |_, _| {},
        tab_shown: |_, _| {},
        commands: &[],
        keyboard: |_| None,
    }
}

/// A view that answers to its own name as the saved mode, and takes the keyboard there.
fn view<P: Made>() -> Entry<Log> {
    let shows = |mode: &str| mode == P::NAME;
    part::<P>(Place::View(idle_view(P::NAME, nothing, shows)))
}

fn rail() -> Sidebar<Log> {
    Sidebar {
        build: nothing,
        focus: |_| Some("rail".to_owned()),
        held_back: |_| 0.0,
    }
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

fn nothing_done(_: &mut Log, _: &Call<'_>, _: &mut rune::Tx) -> Outcome {
    Outcome::Declined
}

const OPEN: &[Action<Log>] = &[Action::new("open", nothing_done)];
const OPEN_TWICE: &[Action<Log>] = &[
    Action::new("open", nothing_done),
    Action::new("close", nothing_done),
    Action::new("open", nothing_done),
];

#[test]
fn a_command_registered_twice_is_refused_unless_by_two_views() {
    let with = |entry: Entry<Log>, commands| Entry { commands, ..entry };
    let cases = [
        (
            vec![with(view::<Columns>(), OPEN), with(view::<Icons>(), OPEN)],
            &[][..],
            Ok(()),
        ),
        (
            vec![
                with(overlay::<Editor>(), OPEN),
                with(overlay::<Menu>(), OPEN),
            ],
            &[][..],
            Err(Fault::CommandTwice("open", "editor", "menu")),
        ),
        (
            vec![with(view::<Columns>(), OPEN), with(overlay::<Menu>(), OPEN)],
            &[][..],
            Err(Fault::CommandTwice("open", "columns", "menu")),
        ),
        (
            vec![with(overlay::<Menu>(), OPEN_TWICE)],
            &[][..],
            Err(Fault::CommandTwice("open", "menu", "menu")),
        ),
        (
            vec![with(overlay::<Menu>(), OPEN)],
            OPEN,
            Err(Fault::CommandTwice("open", "menu", "window")),
        ),
    ];

    for (entries, own, expected) in cases {
        let entries = table(entries);
        let registered = names(entries.iter());
        assert_eq!(Registry::check(entries, own), expected, "{registered:?}");
    }
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
                part::<Rail>(Place::Sidebar(rail())),
                part::<Menu>(Place::Sidebar(rail())),
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
        assert_eq!(Registry::check(entries, &[]), expected, "{registered:?}");
    }
}

#[test]
fn the_parts_this_crate_registers_can_be_a_window() {
    assert_eq!(
        Registry::check(crate::PARTS, crate::commands::own()),
        Ok(())
    );
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
fn a_part_builds_its_element_in_every_slot_but_the_drawer_and_the_strip_which_the_shell_hands_more()
{
    let strip = Strip {
        build: |_, _, _| None,
        new_tab: None,
        shows: |_| false,
    };
    let places = [
        (Place::Tabs(strip), Slot::Tabs, false),
        (Place::Location(nothing), Slot::Location, true),
        (Place::Sidebar(rail()), Slot::Sidebar, true),
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
        assert_eq!(
            registry.drawer().is_some(),
            slot == Slot::Drawer,
            "{slot:?}"
        );
        assert_eq!(registry.strip().is_some(), slot == Slot::Tabs, "{slot:?}");
        assert_eq!(registry.sidebar().is_some(), slot == Slot::Sidebar);
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

fn observed(entries: &'static [Entry<Log>]) -> (Rc<Browser>, Inbox<Log>) {
    let (browser, inbox, _) = observed_with(Registry::new(entries, "columns"));
    (browser, inbox)
}

/// [`observed`] over `registry`, with what says which view hears.
fn observed_with(registry: Registry<'static, Log>) -> (Rc<Browser>, Inbox<Log>, ShownView) {
    let (settings, ids, inbox) = (Preferences::default(), Ids::default(), Inbox::new());
    let worn = Worn::empty();
    let shown = registry.shown_now();
    let setup = Setup {
        settings: &settings,
        ids: &ids,
        worn: &worn,
        // The made-up parts are over a log, and the handle is the window's.
        later: &Inbox::new().later(),
        shown: &shown,
    };
    let browser = Browser::with_preferences(Rc::new(LocalFileSource), ViewPreferences::default());
    inbox.observe(&browser, registry.listeners(&setup));
    (browser, inbox, shown)
}

fn applied(inbox: &Inbox<Log>) -> (Log, Vec<bool>) {
    let mut runtime = Runtime::new(Log::new(), |_, _| el(), (64.0, 64.0), Env::new(true, 1.0));
    let mut asks_for_a_view = Vec::new();
    runtime.update(|log, tx| {
        while let Some(queued) = inbox.pop() {
            asks_for_a_view.push((queued.update)(log, tx));
        }
    });
    (runtime.state, asks_for_a_view)
}

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

#[test]
fn the_view_shown_later_hears_the_browser_in_place_of_the_one_it_replaces() {
    let entries = table(vec![view::<Columns>(), view::<Icons>(), overlay::<Menu>()]);
    let mut registry = Registry::new(entries, "columns");
    let (browser, inbox, shown) = observed_with(registry);

    browser.toggle_hidden();
    assert!(!registry.show("list"));
    assert!(!registry.show("columns"));
    assert!(registry.show("icons"));
    shown.follow(registry);
    browser.toggle_hidden();

    assert_eq!(registry.shown().map(|view| view.focus), Some("icons"));
    assert!(shown.is::<Icons>() && !shown.is::<Columns>());
    assert_eq!(names(registry.active()), ["icons", "menu"]);
    assert_eq!(
        applied(&inbox).0,
        [
            "columns heard true",
            "menu heard true",
            "icons heard false",
            "menu heard false",
        ]
    );
}

#[test]
fn a_weak_way_into_a_queue_says_whether_it_holds_an_update() {
    let inbox = Inbox::<Log>::new();
    let weak = inbox.later().downgrade();
    assert!(!weak.is_pending(), "an empty queue");

    weak.upgrade()
        .expect("the queue is there")
        .then(|log: &mut Log, _| {
            log.push("applied".to_owned());
            false
        });
    assert!(weak.is_pending(), "a queued update");

    assert!(inbox.pop().is_some());
    assert!(!weak.is_pending(), "the update taken");

    inbox.later().then(|_: &mut Log, _| false);
    drop(inbox);
    assert!(!weak.is_live());
    assert!(!weak.is_pending(), "a queue that has gone");
}

#[test]
fn yield_paint_only_triggers_for_staged_publications_and_clears_on_load_finished() {
    crate::test_support::in_own_process(
        "slots::tests::yield_paint_only_triggers_for_staged_publications_and_clears_on_load_finished",
        &[],
        || {
            let small = tempfile::tempdir().expect("small tempdir");
            for i in 0..5 {
                std::fs::write(small.path().join(format!("f{i}.txt")), "").expect("file");
            }
            let large = tempfile::tempdir().expect("large tempdir");
            for i in 0..600 {
                std::fs::write(large.path().join(format!("f{i:04}.txt")), "").expect("file");
            }

            let browser =
                Browser::with_preferences(Rc::new(LocalFileSource), ViewPreferences::default());
            let inbox = Inbox::<Log>::new();
            inbox.observe(&browser, Vec::new());

            let loaded = Rc::new(std::cell::Cell::new(false));
            let replaced_count = Rc::new(std::cell::Cell::new(None::<usize>));
            let loaded_flag = loaded.clone();
            let replaced_flag = replaced_count.clone();
            browser.observe(move |event| match event {
                BrowserEvent::EntriesReplaced { count, .. } => replaced_flag.set(Some(*count)),
                BrowserEvent::LoadFinished { .. } => loaded_flag.set(true),
                _ => {}
            });

            let ctx = glib::MainContext::default();

            // Inline publication: EntriesReplaced + LoadFinished arrive together -> never yields.
            browser.navigate_location(strata_core::model::Location::local(small.path()), false);
            let mut saw_yield = false;
            let deadline = std::time::Instant::now() + crate::test_support::PATIENCE;
            while !loaded.get() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "small directory load timed out"
                );
                let _ = ctx.iteration(true);
                saw_yield |= inbox.take_yield_paint();
            }
            assert_eq!(replaced_count.get(), Some(5));
            assert!(
                !saw_yield,
                "inline publication with LoadFinished must not set yield_paint"
            );

            // Staged publication: initial EntriesReplaced yields while tail chunks remain.
            loaded.set(false);
            replaced_count.set(None);
            browser.navigate_location(strata_core::model::Location::local(large.path()), false);
            let mut yielded_before_finish = false;
            let deadline = std::time::Instant::now() + crate::test_support::PATIENCE;
            while !loaded.get() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "large directory load timed out"
                );
                let _ = ctx.iteration(true);
                if replaced_count.get().is_some() && !loaded.get() {
                    yielded_before_finish |= inbox.take_yield_paint();
                }
            }
            assert!(
                replaced_count.get().is_some_and(|count| count < 600),
                "large directory should publish an initial staged chunk"
            );
            assert!(
                yielded_before_finish,
                "staged publication must set yield_paint before tail chunks drain"
            );
            assert!(
                !inbox.take_yield_paint(),
                "yield_paint must be false once LoadFinished arrives"
            );
        },
    );
}
