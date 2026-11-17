// SPDX-License-Identifier: MIT

//! The transfer the floating Copy to or Move to chooser starts (`crate::chooser::floating`).

use strata_core::model::Location;

use super::begin_transfer;
use crate::App;

/// Copies or moves `sources` into the folder chosen in the floating chooser, revealing what
/// lands there (`finish_copy_move_to` in `src/ui/browser/transfer/destination_dialog.rs`).
pub(crate) fn transfer_to_chosen(
    app: &mut App,
    destination: Location,
    sources: Vec<Location>,
    move_sources: bool,
) {
    begin_transfer(app, destination, sources, move_sources);
}
