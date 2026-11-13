// SPDX-License-Identifier: MIT

//! The Trash's "Empty Trash" button in the Icons and List panes' header, GTK's
//! `empty_trash_button` (`src/ui/browser/pane_header.rs`) as `ContentActions` places it
//! (`src/ui/browser_modes/pane_header.rs`): shown only over the Trash's root, insensitive
//! while it lists nothing (`show_count_controls` in `src/ui/browser_modes/pane_state.rs`),
//! icon-only, named and tooltipped "Empty Trash", and running [`crate::trash`]'s
//! `empty-trash`, which measures, asks and empties as GTK's `confirm_empty_trash` does.

use rune::{El, Tx};
use strata_components::header;
use strata_kit::icons::IconName;

use crate::{
    App, buttons,
    commands::{self, On},
    trash::empty,
};

/// The button for the view keyed `view`, while it shows the Trash's root.
pub(super) fn button(app: &App, view: &str) -> Option<El> {
    let shown = app.browser.active_location()?;
    if shown != empty::root() {
        return None;
    }
    let depth = app.browser.active_depth()?;
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
    Some(buttons::on_return(format!("{view}-empty-trash"), built))
}

#[cfg(test)]
mod tests;
