// SPDX-License-Identifier: MIT

use strata_core::model::Location;

use super::begin_transfer;
use crate::App;

/// Copies or moves `sources` into the folder chosen in the floating chooser.
pub(crate) fn transfer_to_chosen(
    app: &mut App,
    destination: Location,
    sources: Vec<Location>,
    move_sources: bool,
) {
    app.transfer.pending_navigate = Some(destination.clone());
    begin_transfer(app, destination, sources, move_sources);
}

/// A reveal, a failure, a cancellation or a move to the dock: no navigation follows.
pub(super) fn forget_navigation(app: &mut App) {
    app.transfer.pending_navigate = None;
}

/// The foreground transfer completed: the browser goes to the destination it did not reveal.
pub(super) fn navigate_if_unrevealed(app: &mut App) {
    if let Some(destination) = app.transfer.pending_navigate.take() {
        app.browser.navigate(destination);
    }
}
