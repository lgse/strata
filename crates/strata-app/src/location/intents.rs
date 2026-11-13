// SPDX-License-Identifier: MIT

//! What the bar's intents do: GTK's `submit_location`, `cancel_location_edit` and the crumbs'
//! handlers (`src/ui/browser/location.rs`, `location/breadcrumbs.rs`).

use rune::Tx;
use strata_components::location_bar::{Completions, LocationIntent, Polled, Via};
use strata_fs::services::{LocationValidationError, sanitize_uri_credentials};

use crate::{App, shell};

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
fn submit(app: &mut App, text: &str, tx: &mut Tx) {
    shell::publish(app);
    let browser = app.browser.clone();
    let opened = typed(text).and_then(|input| browser.navigate_input_located(&input, None));
    shell::settle(app, tx);
    match opened {
        Ok(_) => give_back(app, tx),
        // GTK mounts a location that needs it, and shows "Unable to open location" for the
        // rest; neither is on this interface yet.
        Err(error) => tracing::warn!(%error, "the typed location did not open"),
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
