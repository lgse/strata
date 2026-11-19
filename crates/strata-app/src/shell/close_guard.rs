// SPDX-License-Identifier: MIT

//! GTK's two close guards, which every close of a window asks: its own Close buttons and
//! closing its last tab (`shell::close_window`), and a close the window manager asks for, which
//! the windowed host keeps open by hand (`crate::host::windowed`, "Refused closes").
//!
//! - **File operations** (`close_guard::install` in `src/ui/window/composition/tabs.rs`): no
//!   window closes while a tab's browser, shown or hidden, has a file operation running.
//! - **Background jobs** (`close_guard::install` in `src/ui/jobs.rs`): the last window does not
//!   close, since the application would end with it, while a custom action's job still runs
//!   or waits.

use crate::{App, custom_actions, error_dialog, shell::tabs};
use strata_core::i18n::tr;

/// Why a window may not close now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Operations,
    Jobs,
}

/// Why `app`'s window may not close, if it may not. `last` is whether the application would
/// end with it.
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

/// Says why the window stays, on GTK's error dialog with GTK's words.
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
