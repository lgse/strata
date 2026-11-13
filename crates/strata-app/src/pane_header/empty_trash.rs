// SPDX-License-Identifier: MIT

//! The Trash's "Empty Trash" button, GTK's `empty_trash_button`
//! (`src/ui/browser/pane_header.rs`): shown only over the Trash's root, insensitive while it
//! lists nothing, icon-only, named and tooltipped "Empty Trash", and running
//! [`crate::trash`]'s `empty-trash`, which measures, asks and empties as GTK's
//! `confirm_empty_trash` does (`show_count_controls` in `src/ui/browser_modes/pane_state.rs`,
//! `update_empty_trash_sensitivity` in `src/ui/browser/columns.rs`). [`super::actions`] places
//! it.

use rune::{El, Tx};
use strata_components::header;
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::{self, On},
    trash::empty,
};

/// The button over the column at `depth`, if it lists the Trash's root.
pub(super) fn built(app: &App, depth: usize) -> Option<El> {
    if app.browser.location_at(depth)? != empty::root() {
        return None;
    }
    let listed = app
        .browser
        .column_entry_counts(depth)
        .is_some_and(|counts| counts.total > 0);
    let look = &app.look;
    let built = header::action(look, IconName::Trash, &app.pane_header.labels.empty_trash)
        .disabled(!listed)
        .on_press_tx(|app: &mut App, tx: &mut Tx| {
            commands::run(app, "empty-trash", On::Focus, tx);
        })
        .build(&look.kit);
    Some(built)
}

#[cfg(test)]
mod tests;
