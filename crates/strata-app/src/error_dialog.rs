// SPDX-License-Identifier: MIT

//! The window's message dialog: an error, a partial failure, or an informational report, as
//! GTK's `src/ui/modal/messages.rs` shows them (`Message::error`, `Message::partial_failure`,
//! `Message::information`), and with an action beside Close as `show_error_dialog_with_action`
//! does. Every failure or post-operation report the window shows goes through it: a rename the
//! browser fails ([`crate::rename_failed`]), a folder or location that cannot be opened
//! ([`crate::open_failed`]), and transfer outcomes ([`crate::transfer`]).
//!
//! It is GTK's message dialog on C15's [`StrataDialog`]: an icon in a toned box, the title,
//! a summary line under it ("The operation could not be completed" by default for errors), and
//! the message as the body.
//!
//! - **An error alone:** the X in a danger box and one Close button, which has the keyboard
//!   when it opens.
//! - **A partial failure (`Completed with errors`):** the X in a danger box, "Some items could
//!   not be processed" under the title, and Close.
//! - **An informational report:** the Info icon in an accent box, the caller's summary under
//!   the title, and Close.
//! - **An error with an action** (Open as Administrator): the action's icon in an accent box,
//!   the action as the primary button with the keyboard, and Close beside it. The action closes
//!   the dialog, then runs.
//!
//! Escape, a click outside and the header's close button close it too. The keyboard goes back
//! where it was. While it is open the window's commands wait, as under any GTK modal layer. A
//! second error while one shows takes its place.

use std::rc::Rc;

use rune::Tx;
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::i18n::tr;
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::Holds,
    shell,
    slots::{Frame, Part, Place, Setup},
};

/// What an action does once the dialog has closed.
type Run = Rc<dyn Fn(&mut App)>;

/// An action offered beside Close: its label, its icon, and what it does.
#[derive(Clone)]
pub(crate) struct Offer {
    label: String,
    icon: IconName,
    run: Run,
}

impl Offer {
    pub(crate) fn new(
        label: impl Into<String>,
        icon: IconName,
        run: impl Fn(&mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            icon,
            run: Rc::new(run),
        }
    }
}

/// One message dialog to show.
#[derive(Clone)]
pub(crate) struct Error {
    title: String,
    summary: Option<String>,
    message: String,
    icon: IconName,
    tone: Tone,
    offer: Option<Offer>,
    /// Where the keyboard goes when the dialog closes, when not where it was as it opened.
    returns_to: Option<&'static str>,
}

impl Error {
    pub(crate) fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            summary: None,
            message: message.into(),
            icon: IconName::Close,
            tone: Tone::Danger,
            offer: None,
            returns_to: None,
        }
    }

    /// GTK's `show_partial_failure_dialog` (`MessageKind::PartialFailure` in `messages.rs`).
    pub(crate) fn partial_failure(message: impl Into<String>) -> Self {
        Self {
            title: tr("Completed with errors"),
            summary: Some(tr("Some items could not be processed")),
            message: message.into(),
            icon: IconName::Close,
            tone: Tone::Danger,
            offer: None,
            returns_to: None,
        }
    }

    /// GTK's `show_information_dialog_with_summary` (`MessageKind::Information` in `messages.rs`).
    pub(crate) fn information(
        title: impl Into<String>,
        summary: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            summary: Some(summary.into()),
            message: message.into(),
            icon: IconName::Info,
            tone: Tone::Accent,
            offer: None,
            returns_to: None,
        }
    }

    /// The same error, offering `offer` beside Close.
    pub(crate) fn offering(mut self, offer: Offer) -> Self {
        self.offer = Some(offer);
        self
    }

    /// The same error, giving the keyboard to the element keyed `key` when it closes: for an
    /// error raised as the keyboard moves there, which the dialog would not see it reach.
    pub(crate) fn returning_to(mut self, key: Option<&'static str>) -> Self {
        self.returns_to = key;
        self
    }
}

/// The dialog's own texts, from the catalogs once.
struct Labels {
    summary: String,
    close: String,
    close_dialog: String,
}

pub(crate) struct ErrorDialog {
    labels: Labels,
    /// The error shown; kept after the dialog closes, for its closing motion.
    shown: Option<Error>,
    open: bool,
}

/// Shows `error` over the window.
pub(crate) fn show(app: &mut App, error: Error) {
    let dialog = &mut app.error_dialog;
    dialog.shown = Some(error);
    dialog.open = true;
}

fn close(app: &mut App) {
    app.error_dialog.open = false;
}

/// The offered action: the dialog closes, then the action runs between `publish` and `settle`.
fn act(app: &mut App, tx: &mut Tx) {
    close(app);
    let Some(run) = app
        .error_dialog
        .shown
        .as_ref()
        .and_then(|error| error.offer.as_ref())
        .map(|offer| offer.run.clone())
    else {
        return;
    };
    shell::publish(app);
    run(app);
    shell::settle(app, tx);
}

fn dialog(app: &App, frame: &mut Frame<'_, App>) -> Option<rune::El> {
    let dialog = &app.error_dialog;
    let error = dialog.shown.as_ref()?;
    let labels = &dialog.labels;
    let subtitle = error
        .summary
        .clone()
        .unwrap_or_else(|| labels.summary.clone());
    let built = StrataDialog::new("error", error.title.clone(), close)
        .open(dialog.open)
        .stacked(app.settings_dialog.is_open())
        .subtitle(subtitle)
        .message(error.message.clone())
        .initial_focus(Focus::Primary)
        .close_label(labels.close_dialog.clone())
        .width(MESSAGE_WIDTH);
    let built = match error.returns_to {
        Some(key) => built.return_focus(key),
        None => built,
    };
    let built = match &error.offer {
        Some(offer) => built
            .icon(offer.icon)
            .tone(Tone::Accent)
            .secondary(labels.close.clone(), close)
            .primary_tx(offer.label.clone(), act),
        None => built
            .icon(error.icon)
            .tone(error.tone)
            .primary(labels.close.clone(), close),
    };
    built.build(&app.look, frame.layer);
    None
}

#[cfg(test)]
pub(crate) fn shown_details(app: &App) -> Option<(IconName, Tone, &str, Option<&str>, &str)> {
    let dialog = &app.error_dialog;
    if !dialog.open {
        return None;
    }
    let error = dialog.shown.as_ref()?;
    Some((
        error.icon,
        error.tone,
        error.title.as_str(),
        error.summary.as_deref(),
        error.message.as_str(),
    ))
}

impl Part for ErrorDialog {
    const PLACE: Place<App> = Place::Dialogs(dialog);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            labels: Labels {
                summary: tr("The operation could not be completed"),
                close: tr("Close"),
                close_dialog: tr("Close dialog"),
            },
            shown: None,
            open: false,
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.error_dialog
            .open
            .then_some(Holds::Modal { field: false })
    }
}

#[cfg(test)]
mod tests;
