// SPDX-License-Identifier: MIT

use crate::{App, custom_actions, error_dialog, shell::tabs};
use strata_core::i18n::tr;

/// Why a window may not close now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Operations,
    Jobs,
}

pub(crate) fn refusal(app: &App, last: bool) -> Option<Refusal> {
    let busy = tabs::tabs(app).iter().any(|&key| tabs::is_busy(app, key));
    if busy {
        Some(Refusal::Operations)
    } else if last && custom_actions::jobs_active(app) {
        Some(Refusal::Jobs)
    } else {
        None
    }
}

pub(crate) fn tell(app: &mut App, refusal: Refusal) {
    match refusal {
        Refusal::Operations => tabs::operations_active(app),
        Refusal::Jobs => error_dialog::show(
            app,
            error_dialog::Error::new(
                tr("Background jobs are still active"),
                tr(
                    "Wait for Jobs to finish, or cancel them in the Jobs dashboard before closing the last window. Cancellation does not undo file changes.",
                ),
            ),
        ),
    }
}

#[cfg(test)]
mod tests;
