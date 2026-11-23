// SPDX-License-Identifier: MIT

use std::cell::Cell;

use rune::{Key as Logical, KeyEvent, Mods, Tx};
use strata_desktop::keys::{Binding, Key, KeyPress};

use super::super::{Action, Call, Outcome, Span, Stage};
use super::routing::{at, f9, note, noted, part};
use strata_core::model::Location;

use super::{MODULE, listed, press};
use crate::{
    App,
    host::offscreen::Offscreen,
    slots::Entry,
    test_support::{
        PATIENCE, folder_of, send_key,
        view::{browser_holds, field, hosted_over_in},
    },
};

thread_local! {
    /// Whether the made-up command takes End.
    static TAKES_END: Cell<bool> = const { Cell::new(false) };
}

fn end(press: &KeyPress) -> bool {
    press.is(Binding::plain(Key::End))
}

/// End is every view's own key: it takes the cursor to the last entry.
fn reads_end(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    note("End, ahead of the view");
    if TAKES_END.get() {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}

fn ahead(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    note("ahead of the view");
    Outcome::Declined
}

fn after(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    note("after the view");
    Outcome::Declined
}

fn last(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    note("unclaimed");
    Outcome::Declined
}

const PROBE: &[Action<App>] = &[
    Action::new("reads-end", reads_end).keys(&[at(Stage::Entry, end)]),
    Action::new("ahead", ahead).keys(&[at(Stage::Dismissal, f9)]),
    Action::new("after", after).keys(&[at(Stage::ItemNavigation, f9)]),
    Action::new("last", last).keys(&[at(Stage::Unclaimed, f9)]),
];

fn the_views_and_a_probe() -> Vec<Entry<App>> {
    struct Probe;
    crate::parts_and(vec![part::<Probe>("probe", PROBE, |_| None)])
}

#[test]
fn the_views_own_keys_stand_where_the_chains_last_stage_does() {
    let ahead: Vec<_> = Stage::ALL
        .iter()
        .filter(|stage| Span::BeforeView.has(**stage))
        .collect();
    let after: Vec<_> = Stage::ALL
        .iter()
        .filter(|stage| Span::AfterView.has(**stage))
        .collect();

    assert_eq!(ahead.first(), Some(&&Stage::TabStrip));
    assert_eq!(ahead.last(), Some(&&Stage::Dismissal));
    assert_eq!(
        after,
        [
            &Stage::ItemNavigation,
            &Stage::AfterYield,
            &Stage::Unclaimed
        ]
    );
}

fn cursor(host: &Offscreen) -> Option<String> {
    browser_holds(host).1
}

fn a_command_reads_a_key_before_the_view_and_the_view_keeps_what_none_takes(
    mode: &'static str,
    name: &str,
) {
    let name = format!("views::{name}");
    hosted_over_in((MODULE, &name), mode, the_views_and_a_probe, |mut host| {
        let _folder = listed(&mut host);
        press(&mut host, Logical::Home, Mods::default());
        let first = cursor(&host);
        assert!(first.is_some(), "Home puts the cursor on the first entry");
        noted();

        TAKES_END.set(true);
        press(&mut host, Logical::End, Mods::default());
        assert_eq!(noted(), ["End, ahead of the view"]);
        assert_eq!(cursor(&host), first, "the command took End from the view");

        TAKES_END.set(false);
        press(&mut host, Logical::End, Mods::default());
        assert_eq!(noted(), ["End, ahead of the view"]);
        assert_ne!(
            cursor(&host),
            first,
            "the view kept the End no command took"
        );

        press(&mut host, Logical::Function(9), Mods::default());
        assert_eq!(
            noted(),
            ["ahead of the view", "after the view", "unclaimed"],
            "each stage hears a key the view leaves once"
        );

        for takes in [true, false] {
            TAKES_END.set(takes);
            for _ in 0..2 {
                let end = KeyEvent {
                    key: Logical::End,
                    mods: Mods::default(),
                };
                send_key(&mut host, end);
            }
            while host.turn() {}
            assert_eq!(noted(), ["End, ahead of the view"; 2], "takes: {takes}");
        }

        let copied = host.app().rows_copied();
        press(&mut host, Logical::Function(5), Mods::default());
        let read_again = host.run_until(PATIENCE, |host| host.app().rows_copied() > copied);
        assert!(read_again, "F5 in the view read nothing");
    });
}

in_each_view! {
    a_command_reads_a_key_before_the_view_and_the_view_keeps_what_none_takes:
    "columns" => a_command_reads_a_key_before_the_columns_and_they_keep_what_none_takes,
    "icons" => a_command_reads_a_key_before_the_grid_and_it_keeps_what_none_takes,
    "list" => a_command_reads_a_key_before_the_table_and_it_keeps_what_none_takes,
}

fn f2(press: &KeyPress) -> bool {
    press.is(Binding::plain(Key::F2))
}

fn after_rename(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    note("F2, after rename");
    Outcome::Declined
}

const AFTER_RENAME: &[Action<App>] =
    &[Action::new("after-rename", after_rename).keys(&[at(Stage::ItemNavigation, f2)])];

fn the_views_and_a_reader_of_f2() -> Vec<Entry<App>> {
    struct Reader;
    crate::parts_and(vec![part::<Reader>("reader", AFTER_RENAME, |_| None)])
}

#[test]
fn f2_with_nothing_to_rename_is_not_taken() {
    let name = "views::f2_with_nothing_to_rename_is_not_taken";
    hosted_over_in(
        (MODULE, name),
        "columns",
        the_views_and_a_reader_of_f2,
        |mut host| {
            let empty = folder_of(&[]);
            host.app().open(Location::local(empty.path()));
            let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, listed), "the folder's listing");
            while host.turn() {}
            noted();

            press(&mut host, Logical::Function(2), Mods::default());

            assert_eq!(noted(), ["F2, after rename"]);
            assert_eq!(field(&host), None, "no editor opened");
        },
    );
}
