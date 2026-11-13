// SPDX-License-Identifier: MIT

//! The headers' "Refresh" button, GTK's `pane_refresh_button` (`src/ui/browser/pane_header.rs`):
//! icon-only, named and tooltipped "Refresh (F5)" for the key as bound now, and reading its
//! column's folder again through B3's `refresh` (`retry_column`), as GTK's does.

use rune::{El, Tx};
use strata_components::header;
use strata_desktop::keys::{Keys, bindings::REFRESH};
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::{self, On},
    location::labels::with_keys,
};

/// The button that refreshes the column at `depth`.
pub(super) fn built(app: &App, depth: usize) -> El {
    let look = &app.look;
    let name = with_keys(
        "Refresh (%{keys})",
        Some("Refresh"),
        Keys::One(REFRESH.shown()),
    );
    header::action(look, IconName::Refresh, &name)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            commands::run(app, "refresh", On::Column(depth), tx);
        })
        .build(&look.kit)
}

#[cfg(test)]
mod tests;
