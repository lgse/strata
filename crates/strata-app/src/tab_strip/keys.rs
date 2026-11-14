// SPDX-License-Identifier: MIT

//! The tab commands, as GTK's `TabWindow::handle_key` reads them: at the window's first
//! stage, from anywhere but a modal layer (the registry cuts the stage there), each taking
//! its key whether or not it did anything. `tab-new` is the header's and the strip's New
//! tab too. `open-in-tab` is read later, from the listing (`open.rs`).

use rune::Tx;
use strata_desktop::keys::{
    KeyPress, Matcher, Modifiers,
    bindings::{
        TAB_CLOSE, TAB_MOVE_LEFT, TAB_MOVE_RIGHT, TAB_NEW, TAB_NEXT, TAB_NUMBERS, TAB_PREVIOUS,
        TAB_REOPEN, TEXT_LARGER, TEXT_SMALLER,
    },
};

use super::{close, new_tab, reopen, select};
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Reads, Stage},
    shell::tabs::{self, shown_tab, tabs},
};

const fn at(matcher: &'static [Matcher]) -> Keys<App> {
    Keys::at(Stage::TabStrip, matcher)
}

/// The tab `delta` places from the one shown, wrapping at either end (`cycle`).
fn cycle(app: &mut App, delta: isize, tx: &mut Tx) {
    let order = tabs(app);
    let Some(at) = order.iter().position(|tab| *tab == shown_tab(app)) else {
        return;
    };
    let next = order[(at as isize + delta).rem_euclid(order.len() as isize) as usize];
    select(app, next, tx);
}

/// The shown tab moved `delta` places, stopping at either end (`move_active_tab`).
fn shift(app: &mut App, delta: isize) {
    let shown = shown_tab(app);
    let order = tabs(app);
    let Some(at) = order.iter().position(|tab| *tab == shown) else {
        return;
    };
    let to = at.checked_add_signed(delta).filter(|to| *to < order.len());
    if let Some(to) = to {
        tabs::move_to(app, shown, to);
    }
}

/// The tab a digit selects, 1 the first and 0 the tenth (`tab_number`, `tab_index`): not a
/// digit that types the `+` or `−` of a text size shortcut, as Shift+1 does on Swiss German.
fn numbered(press: &KeyPress) -> Option<usize> {
    let sizes = TEXT_LARGER
        .primary()
        .is_some_and(|primary| press.is(primary))
        || press.is(TEXT_SMALLER);
    if sizes {
        return None;
    }
    let digit = press.digit_in(TAB_NUMBERS)?;
    Some((usize::from(digit) + 9) % 10)
}

fn number(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    let Some(index) = call.press.and_then(numbered) else {
        return Outcome::Declined;
    };
    if let Some(&key) = tabs(app).get(index) {
        select(app, key, tx);
    }
    Outcome::Ran
}

/// What a tab command did, as its key: taken either way.
fn ran(act: impl FnOnce()) -> Outcome {
    act();
    Outcome::Ran
}

pub(super) const COMMANDS: &[Action<App>] = &[
    Action::new("tab-new", |app, _, tx| ran(|| new_tab(app, tx)))
        .command(TAB_NEW)
        .keys(&[at(&[TAB_NEW.ignoring(Modifiers::empty())])]),
    Action::new("tab-close", |app, _, tx| {
        ran(|| {
            close(app, shown_tab(app), tx);
        })
    })
    .command(TAB_CLOSE)
    .keys(&[at(&[TAB_CLOSE.ignoring(Modifiers::empty())])]),
    Action::new("tab-reopen", |app, _, tx| ran(|| reopen(app, tx)))
        .command(TAB_REOPEN)
        .keys(&[at(&[TAB_REOPEN.ignoring(Modifiers::empty())])]),
    Action::new("tab-next", |app, _, tx| ran(|| cycle(app, 1, tx)))
        .command(TAB_NEXT)
        .keys(&[at(&[TAB_NEXT.ignoring(Modifiers::empty())])]),
    Action::new("tab-previous", |app, _, tx| ran(|| cycle(app, -1, tx)))
        .command(TAB_PREVIOUS)
        .keys(&[at(&[TAB_PREVIOUS.ignoring(Modifiers::empty())])]),
    Action::new("tab-move-left", |app, _, _| ran(|| shift(app, -1)))
        .command(TAB_MOVE_LEFT)
        .keys(&[at(&[TAB_MOVE_LEFT.ignoring(Modifiers::empty())])]),
    Action::new("tab-move-right", |app, _, _| ran(|| shift(app, 1)))
        .command(TAB_MOVE_RIGHT)
        .keys(&[at(&[TAB_MOVE_RIGHT.ignoring(Modifiers::empty())])]),
    Action::new("tab-number", number).keys(&[Keys {
        stage: Stage::TabStrip,
        reads: Reads::Digits(TAB_NUMBERS),
        when: |_, _| true,
    }]),
    super::open::OPEN_IN_TAB,
];
