// SPDX-License-Identifier: MIT

use rune::{Mods, TextEdit, Tx};
use strata_components::{
    contract::dnd::{DropAction, DropOffer},
    location_bar::{LocationIntent, Polled, Via},
};
use strata_core::model::Location;
use strata_fs::{
    app::typed_location_uri,
    services::{LocationValidationError, sanitize_uri_credentials},
};

use crate::{
    App,
    commands::{self, On},
    open_failed, shell,
};

/// One of the bar's intents, or one a call into its state raised.
pub(super) fn heard(app: &mut App, intent: LocationIntent, tx: &mut Tx) {
    match intent {
        LocationIntent::Open { id, via } => open(app, &id, via, tx),
        LocationIntent::OpenInNewTab { id } => open_in_new(app, &id, "open-in-tab", tx),
        LocationIntent::OpenInNewWindow { id } => open_in_new(app, &id, "open-in-window", tx),
        LocationIntent::Submit { text } => submit(app, &text, tx),
        LocationIntent::Cancelled => {
            app.location.typed = None;
            give_back(app, tx);
        }
        LocationIntent::Complete(query) => {
            let later = app.location.later.clone();
            super::completion::complete(app, query, &later, tx);
        }
        LocationIntent::CancelCompletion => app.location.completing.cancel(),
        LocationIntent::CopyPath => {
            if let Some(folder) = app.location.shown.clone() {
                commands::run(app, "copy-path", On::Place(folder), tx);
            }
        }
        LocationIntent::DropInto { id, dropped } => {
            if let Some(folder) = crumb(app, &id) {
                crate::dnd::commit_file_drop(app, folder, &dropped, tx);
            }
        }
        LocationIntent::DropHover { .. } => {}
        other => tracing::debug!(?other, "the location bar asked for what is not ported yet"),
    }
}

pub(super) fn drop_action(
    app: &App,
    id: &str,
    offer: &DropOffer,
    mods: Mods,
) -> Option<DropAction> {
    let folder = crumb(app, id)?;
    crate::dnd::drop_action(app, &folder, offer, mods)
}

/// Routes what a call into the bar's state raised, as the bar routes its own.
pub(super) fn route(app: &mut App, polled: Polled, tx: &mut Tx) {
    for intent in polled.intents {
        heard(app, intent, tx);
    }
}

pub(super) fn route_ticked(app: &mut App, polled: Polled, tx: &mut Tx) {
    for intent in polled.intents {
        match intent {
            LocationIntent::Cancelled if tx.focused().is_some() => app.location.typed = None,
            intent => heard(app, intent, tx),
        }
    }
}

/// The folder of the crumb the bar calls `id`.
fn crumb(app: &App, id: &str) -> Option<Location> {
    let index = id.parse::<usize>().ok()?;
    app.location.crumbs.get(index).cloned()
}

fn open(app: &mut App, id: &str, via: Via, tx: &mut Tx) {
    let Some(folder) = crumb(app, id) else {
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

/// A crumb's middle click: its folder in a new tab or window, by the command `command`.
fn open_in_new(app: &mut App, id: &str, command: &str, tx: &mut Tx) {
    if let Some(folder) = crumb(app, id) {
        commands::run(app, command, On::Place(folder), tx);
    }
}

fn submit(app: &mut App, text: &str, tx: &mut Tx) {
    app.location.state.edit = TextEdit::new("");
    shell::publish(app);
    let browser = app.browser.clone();
    let opened =
        typed(text).and_then(|input| browser.navigate_input_located(&input, None).map(|_| input));
    app.location.typed = opened.as_ref().ok().cloned();
    shell::settle(app, tx);
    give_back(app, tx);
    if let Err(error) = opened {
        open_failed::typed_refused(app, &error);
    }
}

pub(super) fn typed(text: &str) -> Result<String, LocationValidationError> {
    let Some(uri) = typed_location_uri(text).filter(|uri| uri.contains("://")) else {
        return Ok(text.to_owned());
    };
    sanitize_uri_credentials(uri).map(|(sanitized, _)| sanitized)
}

/// The keyboard goes back to the shown view (`browser.focus_active()`).
fn give_back(app: &App, tx: &mut Tx) {
    if let Some(view) = shell::view_focus(app) {
        tx.focus(view, true);
    }
}
