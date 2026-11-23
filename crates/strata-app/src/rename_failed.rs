// SPDX-License-Identifier: MIT

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
