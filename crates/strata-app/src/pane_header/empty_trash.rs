// SPDX-License-Identifier: MIT

//! The Trash's "Empty Trash" button, GTK's `empty_trash_button`
//! (`src/ui/browser/pane_header.rs`): shown only over the Trash's root, insensitive while it
//! lists nothing, icon-only, named and tooltipped "Empty Trash", and running
//! [`crate::trash`]'s `empty-trash`, which measures, asks and empties as GTK's
//! `confirm_empty_trash` does. It goes in two places, as on GTK:
//!
//! - **The Icons and List panes' header**, as `ContentActions` places it
//!   (`src/ui/browser_modes/pane_header.rs`, `show_count_controls` in `pane_state.rs`).
//! - **A Columns column's header** (`column_header` in
//!   `src/ui/browser/columns/append/header.rs`, `update_empty_trash_sensitivity`), through C2's
//!   `header_actions` slot, which asks only for the targeted column, as GTK shows its header
//!   actions; not a focus stop, as the slot asks.

use rune::{El, Tx};
use strata_components::{columns::HeaderCx, header};
use strata_kit::icons::IconName;

use crate::{
    App, buttons,
    commands::{self, On},
    trash::empty,
};

/// The button for the single-pane view keyed `view`, while it shows the Trash's root.
pub(super) fn button(app: &App, view: &str) -> Option<El> {
    let built = built(app, app.browser.active_depth()?)?;
    Some(buttons::on_return(format!("{view}-empty-trash"), built))
}

/// The button in a Columns column's header, while the column lists the Trash's root. C2 asks
/// only for the targeted column's.
pub(crate) fn column_button(app: &App, cx: &HeaderCx) -> Option<El> {
    let built = built(app, cx.depth)?;
    Some(
        built
            .key(format!("columns-empty-trash-{}", cx.depth))
            .focusable(false),
    )
}

/// The button over the column at `depth`, if it lists the Trash's root.
fn built(app: &App, depth: usize) -> Option<El> {
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
