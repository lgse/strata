// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_desktop::keys::bindings::{
    ENTER, ENTER_ANY_SHIFT, ESCAPE, ESCAPE_ANY, ESCAPE_ANY_SHIFT,
};

use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Situation, Stage},
    rename, shell,
};

/// `cancel_rename`: an open rename ends without renaming.
fn rename_cancelled(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    rename::cancel(app, tx);
    Outcome::Ran
}

fn dismissed(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if crate::preview::dismiss_escape(app, tx) {
        return Outcome::Ran;
    }
    let browser = &app.browser;
    if browser.close_peek() || browser.clear_active_selection() {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}

/// `directory_enter_or_escape`'s Escape: the browser closes its deepest column.
fn escaped(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.browser.clone().escape();
    Outcome::Ran
}

fn activated(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if crate::chooser::confirms_focused(app, tx) {
        return Outcome::Ran;
    }
    let browser = app.browser.clone();
    if shows_columns(app) {
        browser.activate_focused();
    } else {
        browser.activate_focused_in_place();
    }
    Outcome::Ran
}

/// The Columns view is shown: the one the saved `browser_mode` names `columns`.
fn shows_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

/// `items_unfocused_for`: an Escape reaches the items only from the shown view's listing.
fn from_the_listing(_: &App, at: &Situation) -> bool {
    at.in_view()
}

pub(crate) const COMMANDS: &[Action<App>] = &[
    Action::new("rename-cancel", rename_cancelled)
        .command(ESCAPE)
        .keys(&[
            Keys::at(Stage::InlineEditing, &[ESCAPE_ANY]).when(|app, _| app.rename.state.is_open())
        ]),
    // `!event.text_has_focus()`.
    Action::new("dismiss", dismissed)
        .command(ESCAPE)
        .keys(&[Keys::at(Stage::Dismissal, &[ESCAPE_ANY_SHIFT])
            .when(|app, at| !app.browser.is_chooser_mode() && !at.text())]),
    Action::new("escape", escaped)
        .command(ESCAPE)
        .keys(&[
            Keys::at(Stage::ItemNavigation, &[ESCAPE_ANY_SHIFT]).when(|app, at| {
                !app.browser.is_chooser_mode()
                    && from_the_listing(app, at)
                    && !shell::preferences(app).tenxer_mode
            }),
        ]),
    Action::new("activate", activated)
        .command(ENTER)
        .keys(&[Keys::at(Stage::ItemNavigation, &[ENTER_ANY_SHIFT]).when(from_the_listing)]),
];

#[cfg(test)]
mod tests;
