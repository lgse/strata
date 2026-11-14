// SPDX-License-Identifier: MIT

//! A modal layer that opens over the files ends the pointer's gesture under it, as GTK's
//! modal layers do: a double click after the layer closes starts a new click run (finding
//! lgse/strata#2454). GTK's dialogs are overlay layers in the window (`src/ui/modal.rs`)
//! and take no `gtk::grab`; what counts is the behaviour driven on GTK.
//! Here a part that starts holding the keyboard as a modal ([`Holds::Modal`]) is that layer,
//! and the shown view ends its gestures ([`View::end_gestures`](crate::slots::View)).

use crate::{App, commands::Holds};

/// Whether a part held the keyboard as a modal at the last poll.
#[derive(Default)]
pub(super) struct Grab {
    modal: bool,
}

/// Ends the shown view's gestures when a modal layer has opened since the last poll.
pub(super) fn follow(app: &mut App) {
    let modal = super::parts(app)
        .active()
        .any(|part| matches!((part.keyboard)(app), Some(Holds::Modal { .. })));
    let opened = modal && !app.window.grab.modal;
    app.window.grab.modal = modal;
    if opened && let Some(view) = super::shown_view(app) {
        (view.end_gestures)(app);
    }
}

#[cfg(test)]
mod tests;
