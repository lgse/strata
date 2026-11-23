// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_core::model::FileEntry;
use strata_desktop::keys::{Modifiers, bindings::OPEN_IN_NEW_TAB};

use super::open_in_new_tab;
use crate::{
    App,
    commands::{self, Action, Call, Keys, On, Outcome, Situation, Stage},
    menus,
};

/// `item_view_has_focus() && !text_has_focus()`.
pub(crate) fn from_the_listing(_: &App, at: &Situation) -> bool {
    at.in_view() && !at.text()
}

pub(crate) enum Focused {
    /// The result under the results' cursor.
    Result(FileEntry),
    /// The browser's cursor entry.
    Entry(FileEntry),
}

pub(crate) fn focused(app: &App) -> Option<Focused> {
    let result = crate::filter::targets::under_cursor(app).map(|(_, entry)| Focused::Result(entry));
    result.or_else(|| app.browser.focused_entry().map(Focused::Entry))
}

fn open_focused(app: &mut App, tx: &mut Tx) -> Outcome {
    match focused(app) {
        None => Outcome::Declined,
        Some(Focused::Result(entry) | Focused::Entry(entry)) if entry.is_directory() => {
            open_in_new_tab(app, entry.location, tx);
            Outcome::Ran
        }
        Some(Focused::Result(file)) => {
            app.browser.open_location(file.location);
            Outcome::Ran
        }
        Some(Focused::Entry(_)) => {
            commands::run(app, "activate", On::Focus, tx);
            Outcome::Ran
        }
    }
}

fn opened(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    if app.browser.is_chooser_mode() {
        return Outcome::Declined;
    }
    match &call.on {
        On::Place(location) => open_in_new_tab(app, location.clone(), tx),
        On::Row((depth, id)) => {
            let Some((_, entries)) = menus::context_entries(app, *depth, *id) else {
                return Outcome::Declined;
            };
            let folders = entries.into_iter().filter(FileEntry::is_directory);
            for folder in folders {
                open_in_new_tab(app, folder.location, tx);
            }
        }
        On::Focus => return open_focused(app, tx),
        // A folder's own menu has no row for it.
        On::Column(_) => return Outcome::Declined,
    }
    Outcome::Ran
}

/// Ctrl wins over Shift, so Ctrl+Shift+Enter opens a tab, not a window.
pub(super) const OPEN_IN_TAB: Action<App> = Action::new("open-in-tab", opened)
    .command(OPEN_IN_NEW_TAB)
    .keys(&[Keys::at(
        Stage::ItemNavigation,
        &[OPEN_IN_NEW_TAB.ignoring(Modifiers::SHIFT_MASK)],
    )
    .when(from_the_listing)]);
