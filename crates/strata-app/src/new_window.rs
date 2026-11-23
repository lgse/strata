// SPDX-License-Identifier: MIT

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
    if app.browser.is_chooser_mode() {
        return Outcome::Declined;
    }
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
