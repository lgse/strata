// SPDX-License-Identifier: MIT

//! What the bar's intents do: GTK's `submit_location`, `cancel_location_edit` and the crumbs'
//! handlers (`src/ui/browser/location.rs`, `location/breadcrumbs.rs`).

use rune::{TextEdit, Tx};
use strata_components::location_bar::{Completions, LocationIntent, Polled, Via};
use strata_fs::services::{LocationValidationError, sanitize_uri_credentials};

use crate::{App, open_failed, shell};

/// One of the bar's intents, or one a call into its state raised.
pub(super) fn heard(app: &mut App, intent: LocationIntent, tx: &mut Tx) {
    match intent {
        LocationIntent::Open { id, via } => open(app, &id, via, tx),
        LocationIntent::Submit { text } => submit(app, &text, tx),
        LocationIntent::Cancelled => give_back(app, tx),
        // No suggestions are listed yet. An answer is still owed: keys the bar holds back
        // while it waits for one (Tab, Enter) run only once it lands.
        LocationIntent::Complete(query) => {
            let answer = Completions::new(query.generation, Vec::new());
            let polled = app.location.state.deliver(answer);
            route(app, polled, tx);
        }
        LocationIntent::CancelCompletion => {}
        other => tracing::debug!(?other, "the location bar asked for what is not ported yet"),
    }
}

/// Routes what a call into the bar's state raised, as the bar routes its own.
pub(super) fn route(app: &mut App, polled: Polled, tx: &mut Tx) {
    for intent in polled.intents {
        heard(app, intent, tx);
    }
}

/// Routes what the bar's tick raised. Its `Cancelled` is a press elsewhere that took the
/// keyboard from the entry, and the keyboard stays where that press put it: GTK cancels in the
/// press's capture phase (`install_location_cancellation`, `src/ui/window/composition/input.rs`)
/// and the pressed widget then takes focus. Only when nothing has the keyboard does it go back
/// to the view. It cannot still be in the bar: the entry has lost it, and the bar's buttons
/// take none.
pub(super) fn route_ticked(app: &mut App, polled: Polled, tx: &mut Tx) {
    for intent in polled.intents {
        match intent {
            LocationIntent::Cancelled if tx.focused().is_some() => {}
            intent => heard(app, intent, tx),
        }
    }
}

/// A crumb: a click or the hierarchy menu opens its folder selecting the one the user came
/// from (`navigate_to_ancestor`); a drag resting on it opens it plainly
/// (`attach_crumb_drop_target`).
fn open(app: &mut App, id: &str, via: Via, tx: &mut Tx) {
    let folder = id
        .parse::<usize>()
        .ok()
        .and_then(|index| app.location.crumbs.get(index))
        .cloned();
    let Some(folder) = folder else {
        return;
    };
    shell::publish(app);
    let browser = app.browser.clone();
    match via {
        Via::Click | Via::Menu => browser.navigate_to_ancestor(folder),
        Via::Spring => browser.navigate_location(folder, false),
    }
    shell::settle(app, tx);
}

/// Enter, Navigate or a chosen suggestion: the typed location opens, and the keyboard goes
/// back to the view. The bar is on its crumbs already.
///
/// A location refused at once, or once the browser has looked (`LocationNavigationRejected`,
/// which [`crate::open_failed`] hears), is reported as "Unable to open location". GTK mounts a
/// location that needs it first; this interface does not mount yet.
fn submit(app: &mut App, text: &str, tx: &mut Tx) {
    // What was typed, a password in a URI among it, is not kept in the closed entry or its
    // undo history.
    app.location.state.edit = TextEdit::new("");
    shell::publish(app);
    let browser = app.browser.clone();
    let opened = typed(text).and_then(|input| browser.navigate_input_located(&input, None));
    shell::settle(app, tx);
    give_back(app, tx);
    if let Err(error) = opened {
        open_failed::typed_refused(app, &error);
    }
}

/// The text to open: a URI loses the credentials typed into it, which GTK hands its mount
/// operation and never keeps with the text (`credentials_from_location_input`).
fn typed(text: &str) -> Result<String, LocationValidationError> {
    if !text.contains("://") {
        return Ok(text.to_owned());
    }
    sanitize_uri_credentials(text).map(|(sanitized, _)| sanitized)
}

/// The keyboard goes back to the shown view (`browser.focus_active()`).
fn give_back(app: &App, tx: &mut Tx) {
    if let Some(view) = shell::view_focus(app) {
        tx.focus(view, true);
    }
}
