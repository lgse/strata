// SPDX-License-Identifier: MIT

//! The dialog a rename the browser fails shows: "Unable to rename item", with the browser's
//! message (`on_rename_failed` in `src/ui/browser/events.rs`, which calls
//! `show_error_dialog` in `src/ui/modal/messages.rs`).
//!
//! It is GTK's error message dialog on C15's [`StrataDialog`]: the X in a danger box, the
//! title, "The operation could not be completed" under it, the message as the body, and one
//! Close button, which has the keyboard when it opens. Escape, a click outside and the
//! header's close button close it too. The keyboard goes back where it was, the view
//! (`rename.rs` has closed its editor by then). While it is open the window's commands wait,
//! as under any GTK modal layer.
//!
//! Only the rename's failure uses it yet. GTK shows the same dialog for other failures; the
//! package that needs one for another command can make this the window's.

use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::i18n::tr;
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::Holds,
    slots::{Frame, Part, Place, Setup},
};

/// The dialog's texts, from the catalogs once.
struct Labels {
    title: String,
    summary: String,
    close: String,
    close_dialog: String,
}

pub(crate) struct RenameFailed {
    labels: Labels,
    /// The browser's message; kept after the dialog closes, for its closing motion.
    message: String,
    open: bool,
}

impl RenameFailed {
    fn close(&mut self) {
        self.open = false;
    }
}

/// Shows the dialog over the window with the browser's `message`.
pub(crate) fn show(app: &mut App, message: String) {
    let failed = &mut app.rename_failed;
    failed.message = message;
    failed.open = true;
}

fn dialog(app: &App, frame: &mut Frame<'_, App>) -> Option<rune::El> {
    let failed = &app.rename_failed;
    let labels = &failed.labels;
    StrataDialog::new("rename-failed", labels.title.clone(), |app: &mut App| {
        app.rename_failed.close();
    })
    .open(failed.open)
    .subtitle(labels.summary.clone())
    .icon(IconName::Close)
    .tone(Tone::Danger)
    .message(failed.message.clone())
    .primary(labels.close.clone(), |app: &mut App| {
        app.rename_failed.close()
    })
    .initial_focus(Focus::Primary)
    .close_label(labels.close_dialog.clone())
    .width(MESSAGE_WIDTH)
    .build(&app.look, frame.layer);
    None
}

impl Part for RenameFailed {
    const PLACE: Place<App> = Place::Dialogs(dialog);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            labels: Labels {
                title: tr("Unable to rename item"),
                summary: tr("The operation could not be completed"),
                close: tr("Close"),
                close_dialog: tr("Close dialog"),
            },
            message: String::new(),
            open: false,
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.rename_failed
            .open
            .then_some(Holds::Modal { field: false })
    }
}

#[cfg(test)]
mod tests;
