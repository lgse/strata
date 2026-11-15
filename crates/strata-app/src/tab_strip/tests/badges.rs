// SPDX-License-Identifier: MIT

//! The strip's number badges while `TAB_NUMBERS.modifiers()` are held alone, including under a
//! modal dialog that cuts `Stage::TabStrip`, and `Ctrl+Enter` stopping on a focused file even
//! when `activate` declines.

use std::cell::Cell;

use rune::{ElementInfo, Key, Mods, Tx};
use strata_desktop::keys::{Modifiers, bindings::OPEN_IN_NEW_TAB};

use super::{OPERATIONS_ACTIVE, act, ctrl, dialog, held, opened, two_tabs_at};
use crate::{
    App,
    columns::Columns,
    commands::{Action, Call, Keys, On, Outcome, Stage, run},
    host::offscreen::Offscreen,
    shell::{self, tabs::tabs},
    slots::{Entry, Part, Place, Setup},
    tab_strip::TabStrip,
    test_support::{
        PATIENCE, fixture, hosted, hosted_over,
        view::{idle, press},
    },
};

const MODULE: &str = "tab_strip::tests::badges";

fn ctrl_shift() -> Mods {
    Mods {
        ctrl: true,
        shift: true,
        ..Mods::default()
    }
}

fn has_ancestor(elements: &[ElementInfo], mut index: Option<usize>, ancestor: usize) -> bool {
    while let Some(at) = index {
        if at == ancestor {
            return true;
        }
        index = elements[at].parent;
    }
    false
}

/// The badge digit drawn inside `tab`, if `tab` shows a badge.
fn badge_under(elements: &[ElementInfo], tab: usize) -> Option<String> {
    let badge = elements.iter().enumerate().find_map(|(index, element)| {
        let is_badge = !element.exiting && element.key.as_deref() == Some("badge");
        (is_badge && has_ancestor(elements, element.parent, tab)).then_some(index)
    })?;
    elements
        .iter()
        .find(|child| !child.exiting && child.parent == Some(badge))
        .and_then(|child| child.text.clone())
}

/// For each tab in the strip, the number badge shown on it now (`None` when its icon shows).
fn tab_badges(host: &mut Offscreen) -> Vec<Option<String>> {
    let elements = host.runtime().elements();
    elements
        .iter()
        .enumerate()
        .filter(|(_, element)| !element.exiting && element.role == "tab")
        .map(|(tab, _)| badge_under(&elements, tab))
        .collect()
}

fn hold(host: &mut Offscreen, mods: Mods) -> Vec<Option<String>> {
    host.modifiers(mods);
    idle(host);
    tab_badges(host)
}

/// Holding Ctrl+Shift without Alt or Super shows 1 to 9 and then 0 on the first ten tabs,
/// leaving tab 11 unbadged; releasing either modifier or losing window focus hides them.
#[test]
fn ctrl_shift_alone_numbers_the_first_ten_tabs_until_released_or_blurred() {
    hosted(
        MODULE,
        "ctrl_shift_alone_numbers_the_first_ten_tabs_until_released_or_blurred",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            for _ in 0..10 {
                let ran = act(&mut host, |app, tx| run(app, "tab-new", On::Focus, tx));
                assert_eq!(ran, Outcome::Ran);
            }
            assert_eq!(tabs(host.app()).len(), 11);
            let none: Vec<Option<String>> = vec![None; 11];
            assert_eq!(tab_badges(&mut host), none, "hidden at rest");

            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            let with_alt = Mods {
                alt: true,
                ..ctrl_shift()
            };
            let with_super = Mods {
                logo: true,
                ..ctrl_shift()
            };
            for mods in [ctrl(), shift, with_alt, with_super] {
                assert_eq!(hold(&mut host, mods), none, "hidden under {mods:?}");
            }

            let mut expected: Vec<Option<String>> =
                (1..=9).chain([0]).map(|n| Some(n.to_string())).collect();
            expected.push(None);
            assert_eq!(hold(&mut host, ctrl_shift()), expected);
            assert_eq!(hold(&mut host, ctrl()), none, "releasing Shift hides them");

            assert_eq!(hold(&mut host, ctrl_shift()), expected);
            host.runtime().window_focus(false);
            idle(&mut host);
            assert_eq!(tab_badges(&mut host), none, "window blur hides them");
        },
    );
}

/// A modal layer cuts `Stage::TabStrip` commands (so Ctrl+T opens nothing), yet GTK shows and
/// hides the tab-number hints before `blocked()`: `badges_shown` reads `shell::held_modifiers`
/// at build time, so they still show and hide under the modal dialog.
#[test]
fn a_modal_dialog_leaves_the_number_badges_live_while_cutting_the_tab_keys() {
    hosted(
        MODULE,
        "a_modal_dialog_leaves_the_number_badges_live_while_cutting_the_tab_keys",
        |mut host| {
            let folder = fixture();
            two_tabs_at(&mut host, folder.path());
            act(&mut host, |app, _| held::busy(&app.browser, "todo.txt"));
            act(&mut host, |app, tx| run(app, "tab-close", On::Focus, tx));
            let said = host.run_until(PATIENCE, |host| dialog(host).is_some());
            assert!(said, "the modal notice opened");
            assert_eq!(dialog(&host).as_deref(), Some(OPERATIONS_ACTIVE));

            press(&mut host, Key::Text("t".to_owned()), ctrl());
            assert_eq!(tabs(host.app()).len(), 2, "the modal cut Ctrl+T");

            assert_eq!(
                hold(&mut host, ctrl_shift()),
                [Some("1".to_owned()), Some("2".to_owned())],
            );
            assert_eq!(hold(&mut host, Mods::default()), [None, None]);
        },
    );
}

thread_local! {
    static UNCLAIMED: Cell<u32> = const { Cell::new(0) };
}

fn saw_unclaimed(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    UNCLAIMED.set(UNCLAIMED.get() + 1);
    Outcome::Ran
}

struct Watch;

impl Part for Watch {
    const PLACE: Place<App> = Place::Overlays(|_, _| None);
    const COMMANDS: &'static [Action<App>] = &[Action::new("watch-ctrl-enter", saw_unclaimed)
        .keys(&[Keys::at(
            Stage::Unclaimed,
            &[OPEN_IN_NEW_TAB.ignoring(Modifiers::SHIFT_MASK)],
        )])];

    fn new(_: &Setup<'_>) -> Self {
        Self
    }
}

/// The Columns view and the tab strip, without `menus` (so `"activate"` is unregistered and
/// declines), plus a watch at `Stage::Unclaimed`.
fn without_activate_and_watched() -> Vec<Entry<App>> {
    vec![
        Entry::of::<Columns>("columns"),
        Entry::of::<TabStrip>("tab_strip"),
        Entry::of::<Watch>("watch"),
    ]
}

/// With no entry focused, Ctrl+Enter declines and goes on to later stages; on a focused file it
/// always stops at `open-in-tab`, even when `"activate"` declines (`open_focused_in_new`).
#[test]
fn ctrl_enter_on_a_file_stops_even_when_activate_declines() {
    hosted_over(
        MODULE,
        "ctrl_enter_on_a_file_stops_even_when_activate_declines",
        without_activate_and_watched,
        |mut host| {
            UNCLAIMED.set(0);
            let empty = tempfile::tempdir().expect("empty folder");
            opened(&mut host, empty.path());
            let view = shell::view_focus(host.app()).expect("the view's key");
            act(&mut host, |_, tx| tx.focus(view, true));
            press(&mut host, Key::Enter, ctrl());
            assert_eq!(UNCLAIMED.get(), 1, "with no focused entry, the key goes on");

            let folder = fixture();
            opened(&mut host, folder.path());
            act(&mut host, |app, _| {
                assert_eq!(run_activate(app), Outcome::Declined);
                app.browser
                    .select_entries_by_name_at(0, &["todo.txt".into()]);
            });
            act(&mut host, |_, tx| tx.focus(view, true));
            press(&mut host, Key::Enter, ctrl());
            assert_eq!(UNCLAIMED.get(), 1, "on a focused file, Ctrl+Enter stopped");
            assert_eq!(tabs(host.app()).len(), 1, "a file opens no tab");
        },
    );
}

fn run_activate(app: &mut App) -> Outcome {
    let mut tx = Tx::default();
    run(app, "activate", On::Focus, &mut tx)
}
