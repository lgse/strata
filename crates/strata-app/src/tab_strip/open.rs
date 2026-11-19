// SPDX-License-Identifier: MIT

//! `open-in-tab`: a folder opened in a new tab at the end, shown, as GTK's window action
//! `open-tab-at` runs `open_in_new_tab` (`src/ui/window/composition/tabs.rs`).
//!
//! - **On a place** (a sidebar row's menu or middle click, a crumb's middle click, a view's
//!   folder row on middle click): its location.
//! - **On a row** (the entry menu's Open In → New Tab): each folder the menu acts on, the
//!   selection when the row is in it, a tab for each, the last shown (`open_entries_in_action`
//!   in `src/ui/browser/context_menu/item_menu.rs`). A file among them opens nothing.
//! - **Ctrl+Enter** (Ctrl+Shift+Enter too), from the listing and not from a field
//!   (`opens_focused_in_new` in `src/ui/window/keyboard/items.rs`, at `item_navigation`): the
//!   result under the results' cursor, else the cursor's entry. A folder opens in a new tab;
//!   a file opens as Enter opens it (`open_focused_in_new`, `activate_focused`). With neither,
//!   the key goes on.
//!
//! A chooser opens no tab on any of them: GTK's `open-tab-at` is a browser window's action,
//! which a chooser's window has none of, and `open_focused_in_new` declines there.
//!
//! 10xer's own Ctrl+Enter (`tenxer_open_focused_in_new`) is A11's.

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

/// What Ctrl+Enter acts on (`selected_search_result().or_else(|| browser.focused_entry())`),
/// and Shift+Enter for a new window (`crate::new_window`).
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

/// Ctrl+Enter: a folder in a new tab; a file as Enter opens it (`activate_focused`), a result
/// at its location.
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
        // GTK's `open_focused_in_new` always stops the key once an entry is focused.
        Some(Focused::Entry(_)) => {
            commands::run(app, "activate", On::Focus, tx);
            Outcome::Ran
        }
    }
}

fn opened(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    // GTK's `open-tab-at` is a browser window's action, which a chooser has none of, and
    // `open_focused_in_new` declines there, so Ctrl+Enter goes on.
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
