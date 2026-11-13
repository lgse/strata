// SPDX-License-Identifier: MIT

//! The window's error dialog: a title and a message, as GTK's `show_error_dialog` shows them
//! (`Message::error` in `src/ui/modal/messages.rs`), and with an action beside Close as its
//! `show_error_dialog_with_action` does. Every failure the window reports goes through it:
//! a rename the browser fails ([`crate::rename_failed`]) and a folder or location that
//! cannot be opened ([`crate::open_failed`]).
//!
//! It is GTK's message dialog on C15's [`StrataDialog`]: an icon in a toned box, the title,
//! "The operation could not be completed" under it, the message as the body.
//!
//! - **An error alone:** the X in a danger box and one Close button, which has the keyboard
//!   when it opens.
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

/// One error to show.
#[derive(Clone)]
pub(crate) struct Error {
    title: String,
    message: String,
    offer: Option<Offer>,
}

impl Error {
    pub(crate) fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            offer: None,
        }
    }

    /// The same error, offering `offer` beside Close.
    pub(crate) fn offering(mut self, offer: Offer) -> Self {
        self.offer = Some(offer);
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
    /// The offered action, pressed and not yet run.
    pending: Option<Run>,
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

/// The offered action: the dialog closes, and the action runs in the next poll, where the
/// browser's answer can be applied (a dialog's button hands its handler no `Tx`).
fn act(app: &mut App) {
    close(app);
    let dialog = &mut app.error_dialog;
    dialog.pending = dialog
        .shown
        .as_ref()
        .and_then(|error| error.offer.as_ref())
        .map(|offer| offer.run.clone());
}

/// Runs the action a press left, as a handler that makes the browser act does.
fn run_pending(app: &mut App, tx: &mut Tx) -> bool {
    let Some(run) = app.error_dialog.pending.take() else {
        return false;
    };
    shell::publish(app);
    run(app);
    shell::settle(app, tx);
    true
}

fn dialog(app: &App, frame: &mut Frame<'_, App>) -> Option<rune::El> {
    let dialog = &app.error_dialog;
    let error = dialog.shown.as_ref()?;
    let labels = &dialog.labels;
    let built = StrataDialog::new("error", error.title.clone(), close)
        .open(dialog.open)
        .subtitle(labels.summary.clone())
        .message(error.message.clone())
        .initial_focus(Focus::Primary)
        .close_label(labels.close_dialog.clone())
        .width(MESSAGE_WIDTH);
    let built = match &error.offer {
        Some(offer) => built
            .icon(offer.icon)
            .tone(Tone::Accent)
            .secondary(labels.close.clone(), close)
            .primary(offer.label.clone(), act),
        None => built
            .icon(IconName::Close)
            .tone(Tone::Danger)
            .primary(labels.close.clone(), close),
    };
    built.build(&app.look, frame.layer);
    None
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
            pending: None,
        }
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        run_pending(app, tx)
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.error_dialog
            .open
            .then_some(Holds::Modal { field: false })
    }
}
