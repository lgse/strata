// SPDX-License-Identifier: MIT

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
        .build(&look.accent_icon_kit());
    Some(built)
}

#[cfg(test)]
mod tests;
