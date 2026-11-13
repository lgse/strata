// SPDX-License-Identifier: MIT

//! The walk over made-up parts: the order commands are asked in, what their answers do, and
//! what each owner of the keyboard leaves of the stages.

use std::{
    any::TypeId,
    cell::{Cell, RefCell},
};

use rune::{El, Key as Logical, KeyEvent, Mods, Tx, el};
use strata_desktop::keys::{Binding, Key, KeyPress, Map, Modifiers, bindings::TAB_NUMBERS};

use super::super::{
    Action, Call, Holds, Keys, On, Outcome, Reads, Situation, Stage, always, on_key, read, run,
};
use super::{MODULE, act, press};
use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    slots::{Entry, Frame, Place},
    test_support::{hosted_over, idle_view},
};

thread_local! {
    /// What the made-up commands said, in the order they were asked.
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// What the made-up owner has of the keyboard.
    static HOLDS: Cell<Option<Holds>> = const { Cell::new(None) };
    /// How often the made-up view has been built.
    static BUILT: Cell<u32> = const { Cell::new(0) };
}

pub(super) fn note(line: impl Into<String>) {
    LOG.with_borrow_mut(|log| log.push(line.into()));
}

pub(super) fn noted() -> Vec<String> {
    LOG.with_borrow_mut(std::mem::take)
}

/// A command that says `$line` and answers `$outcome`.
macro_rules! says {
    ($($name:ident: $line:literal => $outcome:expr;)*) => {
        $(
            fn $name(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
                note($line);
                $outcome
            }
        )*
    };
}

fn f8(press: &KeyPress) -> bool {
    press.is(Binding::plain(Key::F8))
}

pub(super) fn f9(press: &KeyPress) -> bool {
    press.is(Binding::plain(Key::F9))
}

/// `reads` at `stage`, wherever the press lands.
pub(super) const fn at(stage: Stage, reads: fn(&KeyPress) -> bool) -> Keys<App> {
    Keys {
        stage,
        reads: Reads::Press(reads),
        when: always,
    }
}

/// A part with `commands` and nothing to draw. Its type tells two of them apart.
pub(super) fn part<P: 'static>(
    name: &'static str,
    commands: &'static [Action<App>],
    keyboard: fn(&App) -> Option<Holds>,
) -> Entry<App> {
    Entry {
        name,
        kind: TypeId::of::<P>,
        place: Place::Overlays(|_, _| None),
        listen: |_| None,
        publish: |_| {},
        caught_up: |_| {},
        poll: |_, _, _| false,
        switched: |_, _| {},
        commands,
        keyboard,
    }
}

fn made_view(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    BUILT.set(BUILT.get() + 1);
    Some(el().key("made-view").w(40.0).h(20.0).focusable(true))
}

/// A view that takes the keyboard at the key `made-view`.
fn view() -> Entry<App> {
    struct Files;
    Entry {
        place: Place::View(idle_view("made-view", made_view, |_| true)),
        ..part::<Files>("made-view", &[], |_| None)
    }
}

fn key(key: Logical) -> KeyEvent {
    KeyEvent {
        key,
        mods: Mods::default(),
    }
}

says! {
    runs_late: "first runs late" => Outcome::Ran;
    declines_early: "first declines early" => Outcome::Declined;
    leaves: "first leaves" => Outcome::Left;
    runs_early: "second runs early" => Outcome::Ran;
    after_a_left: "second runs after the key was left" => Outcome::Ran;
}

/// Says which tab digit the press is.
fn digit(_: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let digit = call.press.and_then(|press| press.digit_in(TAB_NUMBERS));
    note(format!("tab {digit:?}"));
    Outcome::Ran
}

const FIRST: &[Action<App>] = &[
    Action::new("late", runs_late).keys(&[at(Stage::ItemNavigation, f9)]),
    Action::new("early-declines", declines_early).keys(&[at(Stage::WindowCommands, f9)]),
    Action::new("leaves", leaves).keys(&[at(Stage::Entry, f8)]),
    Action::new("digit", digit).keys(&[Keys {
        stage: Stage::TabStrip,
        reads: Reads::Digits(TAB_NUMBERS),
        when: always,
    }]),
];

const SECOND: &[Action<App>] = &[
    Action::new("early", runs_early).keys(&[at(Stage::WindowCommands, f9)]),
    Action::new("after-left", after_a_left).keys(&[at(Stage::ItemNavigation, f8)]),
];

fn two_tables() -> Vec<Entry<App>> {
    struct First;
    struct Second;
    vec![
        view(),
        part::<First>("first", FIRST, |_| None),
        part::<Second>("second", SECOND, |_| None),
    ]
}

/// GTK's chain: a press is the first stage's that reads it, a stage that declines hands it
/// on, and one that leaves it to the focused widget ends the chain.
#[test]
fn a_press_is_asked_by_stage_then_by_registration_until_one_runs_or_leaves_it() {
    hosted_over(
        MODULE,
        "routing::a_press_is_asked_by_stage_then_by_registration_until_one_runs_or_leaves_it",
        two_tables,
        |mut host| {
            press(&mut host, Logical::Function(9), Mods::default());
            assert_eq!(noted(), ["first declines early", "second runs early"]);

            press(&mut host, Logical::Function(8), Mods::default());
            assert_eq!(noted(), ["first leaves"]);

            let held = Mods {
                shift: true,
                cmd: true,
                ctrl: true,
                ..Mods::default()
            };
            press(&mut host, Logical::Command('3'), held);
            assert_eq!(noted(), ["tab Some(3)"]);

            let by_name = act(&mut host, |app, tx| run(app, "late", On::Focus, tx));
            assert_eq!(
                (by_name, noted()),
                (Outcome::Ran, vec!["first runs late".to_owned()])
            );
        },
    );
}

says! {
    tab_strip: "tab strip" => Outcome::Declined;
    entry: "entry" => Outcome::Declined;
    zoom: "zoom" => Outcome::Declined;
    default_tab: "default tab" => Outcome::Declined;
    inline_editing: "inline editing" => Outcome::Declined;
    text_input: "text input" => Outcome::Declined;
    outside_text: "file commands, outside text" => Outcome::Declined;
    in_the_filter: "file commands, in the filter" => Outcome::Declined;
    after_yield: "after yield" => Outcome::Declined;
    owners_after_yield: "after yield, the owner's" => Outcome::Declined;
    unclaimed: "unclaimed" => Outcome::Declined;
}

const OWNER: &[Action<App>] =
    &[Action::new("own", owners_after_yield).keys(&[at(Stage::AfterYield, f9)])];

/// A command at each stage the owners' rules tell apart, each of which says so and declines.
const STAGES: &[Action<App>] = &[
    Action::new("tab-strip", tab_strip).keys(&[at(Stage::TabStrip, f9)]),
    Action::new("entry", entry).keys(&[at(Stage::Entry, f9)]),
    Action::new("zoom", zoom).keys(&[at(Stage::ZoomShortcut, f9)]),
    Action::new("default-tab", default_tab).keys(&[at(Stage::DefaultTabNavigation, f9)]),
    Action::new("inline-editing", inline_editing).keys(&[at(Stage::InlineEditing, f9)]),
    Action::new("text-input", text_input).keys(&[at(Stage::TextInput, f9)]),
    Action::new("outside-text", outside_text)
        .keys(&[at(Stage::FileCommands, f9).when(|_, at| !at.text())]),
    Action::new("in-the-filter", in_the_filter)
        .keys(&[at(Stage::FileCommands, f9).when(|_, at| at.filter())]),
    Action::new("after-yield", after_yield).keys(&[at(Stage::AfterYield, f9)]),
    Action::new("unclaimed", unclaimed).keys(&[at(Stage::Unclaimed, f9)]),
];

fn an_owner_and_every_stage() -> Vec<Entry<App>> {
    struct Owner;
    struct Stages;
    vec![
        view(),
        part::<Owner>("owner", OWNER, |_| HOLDS.get()),
        part::<Stages>("stages", STAGES, |_| None),
    ]
}

/// The GTK dispatcher's owners of input, row by row: how far the chain runs, whether the tab
/// strip's commands do, and whether a window action runs or only takes its key
/// (`input_owner.rs`, `commands.rs:85` and `:177`, `composition/tabs.rs:760`,
/// `composition.rs:239`).
#[test]
fn each_owner_of_the_keyboard_leaves_the_stages_gtk_leaves() {
    const HEAD: [&str; 3] = ["tab strip", "entry", "zoom"];
    const TAIL: [&str; 3] = ["after yield, the owner's", "after yield", "unclaimed"];
    let chain =
        |stages: &[&'static str]| -> Vec<&'static str> { [&HEAD[..], stages, &TAIL[..]].concat() };
    let cases: [(Option<Holds>, Vec<&str>, bool); 9] = [
        (
            None,
            chain(&[
                "default tab",
                "inline editing",
                "text input",
                "file commands, outside text",
            ]),
            false,
        ),
        // A plain popover takes every key: no stage hears it (GTK, driven).
        (Some(Holds::Popover), vec![], false),
        (
            Some(Holds::Field),
            chain(&["default tab", "inline editing", "text input"]),
            false,
        ),
        (
            Some(Holds::Filter),
            chain(&[
                "default tab",
                "inline editing",
                "text input",
                "file commands, in the filter",
            ]),
            false,
        ),
        (
            Some(Holds::Location),
            chain(&["default tab", "inline editing", "text input"]),
            false,
        ),
        // The first window action takes the key, and nothing after it is asked.
        (
            Some(Holds::Edit),
            vec![
                "tab strip",
                "entry",
                "zoom",
                "default tab",
                "inline editing",
            ],
            true,
        ),
        (Some(Holds::Menu), chain(&[]), false),
        // The tab strip is blocked, the layer's own action runs, and the next one takes the key.
        (
            Some(Holds::Modal { field: false }),
            vec!["entry", "zoom", "after yield, the owner's"],
            true,
        ),
        (
            Some(Holds::Modal { field: true }),
            vec!["entry", "zoom", "after yield, the owner's"],
            true,
        ),
    ];
    hosted_over(
        MODULE,
        "routing::each_owner_of_the_keyboard_leaves_the_stages_gtk_leaves",
        an_owner_and_every_stage,
        move |mut host| {
            for (holds, asked, taken) in cases {
                HOLDS.set(holds);
                let event = key(Logical::Function(9));

                let took = act(&mut host, move |app, tx| on_key(app, &event, tx));

                assert_eq!(
                    (noted(), took),
                    (asked.iter().map(|s| (*s).to_owned()).collect(), taken),
                    "{holds:?}"
                );
            }
        },
    );
}

/// F9 with or without Ctrl or Alt.
fn f9_held(press: &KeyPress) -> bool {
    press.is(Binding::plain(Key::F9).ignoring(Modifiers::CONTROL_MASK.union(Modifiers::ALT_MASK)))
}

says! {
    window_command: "window command" => Outcome::Declined;
    item_navigation: "item navigation" => Outcome::Declined;
    left_unclaimed: "unclaimed" => Outcome::Declined;
}

const HELD: &[Action<App>] = &[
    Action::new("window-command", window_command).keys(&[at(Stage::WindowCommands, f9_held)]),
    Action::new("item-navigation", item_navigation).keys(&[at(Stage::ItemNavigation, f9_held)]),
    Action::new("unclaimed", left_unclaimed).keys(&[at(Stage::Unclaimed, f9_held)]),
];

fn an_owner_and_held_keys() -> Vec<Entry<App>> {
    struct Owner;
    struct Held;
    vec![
        view(),
        part::<Owner>("owner", &[], |_| HOLDS.get()),
        part::<Held>("held", HELD, |_| None),
    ]
}

/// GTK's plain popover takes every key while focus is in it, with Ctrl or Alt held as
/// without (driven: Alt+Up and Ctrl+Alt+= do nothing there): no stage hears one.
#[test]
fn a_popover_takes_every_key_with_or_without_modifiers() {
    const ALL: [&str; 3] = ["window command", "item navigation", "unclaimed"];
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    let cases: [(Option<Holds>, Mods, &[&str]); 4] = [
        (None, alt, &ALL),
        (Some(Holds::Popover), Mods::default(), &[]),
        (Some(Holds::Popover), ctrl, &[]),
        (Some(Holds::Popover), alt, &[]),
    ];
    hosted_over(
        MODULE,
        "routing::a_popover_takes_every_key_with_or_without_modifiers",
        an_owner_and_held_keys,
        move |mut host| {
            for (holds, mods, asked) in cases {
                HOLDS.set(holds);
                let event = KeyEvent {
                    key: Logical::Function(9),
                    mods,
                };

                act(&mut host, move |app, tx| on_key(app, &event, tx));

                assert_eq!(noted(), asked, "{holds:?}, {mods:?}");
            }
        },
    );
}

fn made_button(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    Some(el().key("made-button").w(40.0).h(20.0).focusable(true))
}

fn an_owner_every_stage_and_a_button() -> Vec<Entry<App>> {
    struct Button;
    let button = Entry {
        place: Place::Overlays(made_button),
        ..part::<Button>("made-button", &[], |_| None)
    };
    let mut parts = an_owner_and_every_stage();
    parts.push(button);
    parts
}

/// Rune hears a key at the root element when an element has the keyboard and at the window
/// when none has or a part holds it. Wherever it lands, each stage hears the press once: a
/// view that did not put the registry ahead of its keys (`commands::in_view`) loses no
/// stage, since the root then asks them all.
#[test]
fn a_press_is_offered_once_from_wherever_the_keyboard_is() {
    const WHOLE: [&str; 10] = [
        "tab strip",
        "entry",
        "zoom",
        "default tab",
        "inline editing",
        "text input",
        "file commands, outside text",
        "after yield, the owner's",
        "after yield",
        "unclaimed",
    ];
    // A field's part holds the keyboard: the window asks, with what a field leaves.
    let under_a_field = [&WHOLE[..6], &WHOLE[7..]].concat();
    let cases: [(Option<&str>, Option<Holds>, &[&str]); 4] = [
        (None, None, &WHOLE),
        (Some("made-button"), None, &WHOLE),
        (Some("made-view"), None, &WHOLE),
        (Some("made-button"), Some(Holds::Field), &under_a_field),
    ];
    hosted_over(
        MODULE,
        "routing::a_press_is_offered_once_from_wherever_the_keyboard_is",
        an_owner_every_stage_and_a_button,
        move |mut host: Offscreen| {
            for (focused, holds, asked) in cases {
                HOLDS.set(holds);
                act(&mut host, |_, tx| match focused {
                    Some(key) => tx.focus(key, false),
                    None => tx.blur(),
                });

                press(&mut host, Logical::Function(9), Mods::default());

                assert_eq!(noted(), asked, "{focused:?}, {holds:?}");
            }
        },
    );
}

/// `context_of` in `src/ui/keys/alias.rs` counts any focused editable, so a letter typed in a
/// modal layer's field is text, where beside the field it is the arrow it stands for.
#[test]
fn a_letter_is_text_in_any_field_and_an_arrow_elsewhere() {
    let cases = [
        (None, true),
        (Some(Holds::Field), false),
        // In a popover, as `in_popover` in `context_of` has it.
        (Some(Holds::Popover), false),
        (Some(Holds::Modal { field: false }), true),
        (Some(Holds::Modal { field: true }), false),
    ];
    hosted_over(
        MODULE,
        "routing::a_letter_is_text_in_any_field_and_an_arrow_elsewhere",
        an_owner_and_every_stage,
        move |mut host| {
            act(&mut host, |app, _| {
                shell::saved(app).change(|settings| settings.type_to_search = false);
            });
            for (holds, an_arrow) in cases {
                HOLDS.set(holds);

                let press = act(&mut host, |app, tx| {
                    let at = Situation::of(app, tx);
                    read(app, &key(Logical::Text("j".to_owned())), &at, tx)
                });

                // A binding on Down matches `j` only once the aliases read it as Down.
                assert_eq!(press.is(Binding::plain(Key::Down)), an_arrow, "{holds:?}");
            }
        },
    );
}

/// The GTK application reads its preference manager at each press, wherever the press lands.
#[test]
fn a_press_in_a_field_is_read_with_the_map_the_settings_select_now() {
    hosted_over(
        MODULE,
        "routing::a_press_in_a_field_is_read_with_the_map_the_settings_select_now",
        an_owner_and_every_stage,
        |mut host| {
            HOLDS.set(Some(Holds::Field));

            let map = act(&mut host, |app, tx| {
                shell::saved(app).change(|settings| settings.tenxer_mode = true);
                let at = Situation::of(app, tx);
                read(app, &key(Logical::Function(9)), &at, tx);
                Map::active()
            });

            assert_eq!(map, Map::Tenxer);
        },
    );
}

/// Queues an update as a service's callback does, and finds the press is not its own.
fn queues_and_declines(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.later().then(|_, _| {
        note("applied");
        true
    });
    Outcome::Declined
}

const QUEUES: &[Action<App>] =
    &[Action::new("queues", queues_and_declines).keys(&[at(Stage::Entry, f8)])];

fn a_view_and_one_that_queues() -> Vec<Entry<App>> {
    struct Queues;
    vec![view(), part::<Queues>("queues", QUEUES, |_| None)]
}

/// Rune commits nothing for a key no handler took, so what is in the queue then is applied
/// by the frame's `poll`, whose answer runs the view.
#[test]
fn what_was_queued_when_a_command_declined_still_runs_the_view() {
    hosted_over(
        MODULE,
        "routing::what_was_queued_when_a_command_declined_still_runs_the_view",
        a_view_and_one_that_queues,
        |mut host: Offscreen| {
            while host.turn() {}
            let built = BUILT.get();

            press(&mut host, Logical::Function(8), Mods::default());

            assert_eq!(noted(), ["applied"]);
            assert!(BUILT.get() > built, "the view did not run for the update");
        },
    );
}

says! {
    in_the_view: "the view has the keyboard" => Outcome::Declined;
    in_a_made_part: "a made-up part has the keyboard" => Outcome::Declined;
    in_a_longer_name: "a part whose name only starts the key has the keyboard" => Outcome::Declined;
}

const WHERE: &[Action<App>] = &[
    Action::new("in-view", in_the_view)
        .keys(&[at(Stage::ItemNavigation, f9).when(|_, at: &Situation| at.in_view())]),
    Action::new("within", in_a_made_part)
        .keys(&[at(Stage::ItemNavigation, f9).when(|_, at: &Situation| at.within("made"))]),
    // `made-view` is not a key of a part named `mad`.
    Action::new("within-a-longer-name", in_a_longer_name)
        .keys(&[at(Stage::ItemNavigation, f9).when(|_, at: &Situation| at.within("mad"))]),
];

fn a_view_and_guards() -> Vec<Entry<App>> {
    struct Guards;
    vec![view(), part::<Guards>("guards", WHERE, |_| None)]
}

/// GTK's guards ask where the keyboard is (`item_view_has_focus`, `sidebar.contains`).
#[test]
fn a_guard_can_ask_whether_the_view_or_a_parts_element_has_the_keyboard() {
    hosted_over(
        MODULE,
        "routing::a_guard_can_ask_whether_the_view_or_a_parts_element_has_the_keyboard",
        a_view_and_guards,
        |mut host: Offscreen| {
            press(&mut host, Logical::Function(9), Mods::default());
            assert_eq!(noted(), Vec::<String>::new(), "nothing has the keyboard");

            act(&mut host, |app, tx| {
                let view = shell::view_focus(app).expect("a view is shown");
                tx.focus(view, false);
            });
            press(&mut host, Logical::Function(9), Mods::default());
            assert_eq!(
                noted(),
                [
                    "the view has the keyboard",
                    "a made-up part has the keyboard"
                ]
            );
        },
    );
}
