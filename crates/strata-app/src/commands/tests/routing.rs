// SPDX-License-Identifier: MIT

//! The walk over made-up parts: the order commands are asked in, what their answers do, and
//! what each owner of the keyboard leaves of the stages.

use std::{
    any::TypeId,
    cell::{Cell, RefCell},
};

use rune::{El, Key as Logical, KeyEvent, Mods, Tx, el};
use strata_desktop::keys::{Binding, Key, KeyPress, bindings::TAB_NUMBERS};

use super::super::{
    Action, Call, Holds, Keys, On, Outcome, Reads, Situation, Stage, always, on_key, run,
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
}

fn note(line: impl Into<String>) {
    LOG.with_borrow_mut(|log| log.push(line.into()));
}

fn noted() -> Vec<String> {
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

fn f9(press: &KeyPress) -> bool {
    press.is(Binding::plain(Key::F9))
}

/// `reads` at `stage`, wherever the press lands.
const fn at(stage: Stage, reads: fn(&KeyPress) -> bool) -> Keys<App> {
    Keys {
        stage,
        reads: Reads::Press(reads),
        when: always,
    }
}

/// A part with `commands` and nothing to draw. Its type tells two of them apart.
fn part<P: 'static>(
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
        commands,
        keyboard,
    }
}

fn made_view(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
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
    let cases: [(Option<Holds>, Vec<&str>, bool); 7] = [
        (
            None,
            chain(&[
                "inline editing",
                "text input",
                "file commands, outside text",
            ]),
            false,
        ),
        (
            Some(Holds::Field),
            chain(&["inline editing", "text input"]),
            false,
        ),
        (
            Some(Holds::Filter),
            chain(&[
                "inline editing",
                "text input",
                "file commands, in the filter",
            ]),
            false,
        ),
        (
            Some(Holds::Location),
            chain(&["inline editing", "text input"]),
            false,
        ),
        // The first window action takes the key, and nothing after it is asked.
        (
            Some(Holds::Edit),
            vec!["tab strip", "entry", "zoom", "inline editing"],
            true,
        ),
        (Some(Holds::Menu), chain(&[]), false),
        // The tab strip is blocked, the layer's own action runs, and the next one takes the key.
        (
            Some(Holds::Modal),
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

says! {
    in_the_view: "the view has the keyboard" => Outcome::Declined;
    in_a_made_part: "a made-up part has the keyboard" => Outcome::Declined;
}

const WHERE: &[Action<App>] = &[
    Action::new("in-view", in_the_view)
        .keys(&[at(Stage::ItemNavigation, f9).when(|_, at: &Situation| at.in_view())]),
    Action::new("within", in_a_made_part)
        .keys(&[at(Stage::ItemNavigation, f9).when(|_, at: &Situation| at.within("made-"))]),
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
