// SPDX-License-Identifier: MIT

//! The dialog a rename the browser fails shows: "Unable to rename item", with the browser's
//! message (`on_rename_failed` in `src/ui/browser/events.rs`, which calls
//! `show_error_dialog` in `src/ui/modal/messages.rs`), on the window's error dialog
//! ([`crate::error_dialog`]). The keyboard goes back to the view: `rename.rs` has closed its
//! editor by then.

use strata_core::i18n::tr;

use crate::{
    App,
    error_dialog::{self, Error},
};

/// Shows the dialog over the window with the browser's `message`.
pub(crate) fn show(app: &mut App, message: String) {
    error_dialog::show(app, Error::new(tr("Unable to rename item"), message));
}

#[cfg(test)]
mod tests;
