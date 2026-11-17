// SPDX-License-Identifier: MIT

//! Revealing the items a paste or duplicate placed once their destination folder lists them,
//! matching `src/ui/browser/events/reveal.rs`.

use rune::Tx;
use strata_core::model::Location;

use crate::{App, filter, shell};

const MAX_BULK_REVEAL_SELECTION: usize = 64;

#[derive(Clone, Debug)]
pub(super) struct PendingReveal {
    destination: Location,
    locations: Vec<Location>,
}

fn shows_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

fn prepare_reveal(app: &mut App, directory: &Location, tx: &mut Tx) {
    app.transfer.pending_reveal = None;
    if let Some(depth) = app.browser.open_depth(directory) {
        filter::clear_for_new_item(app, depth, tx);
    }
}

/// Selects `locations`, new in `destination`, once it lists them: what a transfer placed, or
/// what a content drop saved (`dnd::content`).
pub(crate) fn apply(
    app: &mut App,
    destination: Location,
    mut locations: Vec<Location>,
    tx: &mut Tx,
) {
    if locations.len() > MAX_BULK_REVEAL_SELECTION {
        locations.truncate(1);
    }
    if let Some(depth) = app.browser.open_depth(&destination) {
        reveal_in_open_folder(app, (depth, destination), locations, tx);
        return;
    }
    let parent_depth = shows_columns(app)
        .then(|| {
            destination
                .parent()
                .and_then(|parent| app.browser.open_depth(&parent))
        })
        .flatten();
    match parent_depth {
        Some(parent_depth) => {
            app.transfer.pending_reveal = None;
            app.browser
                .descend_revealing(parent_depth, destination, locations);
        }
        None => {
            prepare_reveal(app, &destination, tx);
            app.browser.reveal_locations(destination, locations);
        }
    }
}

fn reveal_in_open_folder(
    app: &mut App,
    (depth, destination): (usize, Location),
    locations: Vec<Location>,
    tx: &mut Tx,
) {
    prepare_reveal(app, &destination, tx);
    if !app.browser.column_has_monitor(depth) {
        app.browser
            .reveal_reloaded_locations(&destination, locations);
        return;
    }
    app.browser
        .reveal_monitored_locations(destination.clone(), &locations);
    if !app.browser.lists_every_target(depth, &locations) {
        app.transfer.pending_reveal = Some(PendingReveal {
            destination,
            locations,
        });
    }
}

pub(super) fn on_listing_changed(app: &mut App, depth: usize) {
    let Some(pending) = app.transfer.pending_reveal.clone() else {
        return;
    };
    if app.browser.location_at(depth).as_ref() != Some(&pending.destination) {
        return;
    }
    app.browser
        .select_entries_by_location_at(depth, &pending.locations);
    if app.browser.lists_every_target(depth, &pending.locations) {
        app.transfer.pending_reveal = None;
    }
}
