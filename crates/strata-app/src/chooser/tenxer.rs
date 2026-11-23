// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_core::{i18n::tr, model::Location};

use super::{accept, keys::UNAVAILABLE};
use crate::App;

const LOCAL_ONLY: &str = "Only local folders can be opened here";

/// `g+` and `g-`: refused in a chooser, with the flash.
pub(crate) fn refuses_pin(app: &mut App, tx: &mut Tx) -> bool {
    if !app.chooser.is_active() {
        return false;
    }
    crate::tenxer::show_flash(app, tr(UNAVAILABLE), tx);
    true
}

pub(crate) fn refuses_place(app: &mut App, location: &Location, tx: &mut Tx) -> bool {
    let opens_here = location.native_path().is_some() || location.is_recent_root();
    if !app.chooser.is_active() || opens_here {
        return false;
    }
    crate::tenxer::show_flash(app, tr(LOCAL_ONLY), tx);
    true
}

pub(crate) fn keeps_load_cursor(app: &App) -> bool {
    app.chooser.is_active() && app.browser.selection_is_load_cursor()
}

/// Escape with nothing left to dismiss cancels a chooser's request (`tenxer_escape`).
pub(crate) fn cancel_on_escape(app: &mut App, tx: &mut Tx) {
    if app.chooser.is_active() {
        accept::cancel(app, tx);
    }
}
