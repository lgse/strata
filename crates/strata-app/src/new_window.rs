// SPDX-License-Identifier: MIT

//! `open-in-window`: a folder opened in a new window of the process, as GTK's window action
//! `open-window-at` runs `open_in_new_window` (`src/ui/window/composition/tabs.rs`), which
//! presents a window on the location as a launch's argument does
//! (`present_location_window` in `src/ui/window/open_argument.rs`). It takes the same places
//! as `open-in-tab` (`crate::tab_strip::open`):
//!
//! - **On a place** (a crumb's or a folder row's Shift or Ctrl middle click): its location.
//! - **On a row** (the entry menu's Open In → New Window): a window for each folder the menu
//!   acts on (`open_entries_in_action` in `src/ui/browser/context_menu/item_menu.rs`). A file
//!   among them opens nothing.
//! - **Shift+Enter**, from the listing and not from a field (`opens_focused_in_new` in
//!   `src/ui/window/keyboard/items.rs`): the result under the results' cursor, else the
//!   cursor's entry. A folder opens in a new window; a file opens as Enter opens it. A chooser
//!   opens no window (`open_focused_in_new` declines there).
//!
//! The window is asked for through the process's queue (`application::ask_for_window`), which
//! the windowed host and the QA host open from, so it leaves `tabs.toml` alone.

use rune::{El, Tx};
use strata_core::model::FileEntry;
use strata_desktop::keys::{Modifiers, bindings::OPEN_IN_NEW_WINDOW};

use crate::{
    App, application,
    commands::{self, Action, Call, Keys, On, Outcome, Situation, Stage},
    menus,
    slots::{Frame, Part, Place, Scope, Setup},
    tab_strip::open::{Focused, focused, from_the_listing},
};

/// The part that registers `open-in-window`; it draws nothing.
pub(crate) struct NewWindow;

fn nothing(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    None
}

impl Part for NewWindow {
    const PLACE: Place<App> = Place::Overlays(nothing);
    const SCOPE: Scope = Scope::Window;
    const COMMANDS: &'static [Action<App>] = &[OPEN_IN_WINDOW];

    fn new(_: &Setup<'_>) -> Self {
        Self
    }
}

/// From the listing, in a window that is not a chooser.
fn listing_of_a_browser(app: &App, at: &Situation) -> bool {
    from_the_listing(app, at) && !app.browser.is_chooser_mode()
}

fn opened(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    match &call.on {
        On::Place(location) => application::ask_for_window(location.clone()),
        On::Row((depth, id)) => {
            let Some((_, entries)) = menus::context_entries(app, *depth, *id) else {
                return Outcome::Declined;
            };
            for folder in entries.into_iter().filter(FileEntry::is_directory) {
                application::ask_for_window(folder.location);
            }
        }
        On::Focus => return open_focused(app, tx),
        // A folder's own menu has no row for it.
        On::Column(_) => return Outcome::Declined,
    }
    Outcome::Ran
}

/// Shift+Enter: a folder in a new window; a file as Enter opens it, a result at its location.
fn open_focused(app: &mut App, tx: &mut Tx) -> Outcome {
    match focused(app) {
        None => Outcome::Declined,
        Some(Focused::Result(entry) | Focused::Entry(entry)) if entry.is_directory() => {
            application::ask_for_window(entry.location);
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

pub(crate) const OPEN_IN_WINDOW: Action<App> = Action::new("open-in-window", opened)
    .command(OPEN_IN_NEW_WINDOW)
    .keys(&[Keys::at(
        Stage::ItemNavigation,
        &[OPEN_IN_NEW_WINDOW.ignoring(Modifiers::empty())],
    )
    .when(listing_of_a_browser)]);

#[cfg(test)]
mod tests;
