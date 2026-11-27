// SPDX-License-Identifier: MIT

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

/// What an offered action does once the dialog has closed.
type Run = Rc<dyn Fn(&mut App)>;

/// What a button action does once the dialog has closed, with the press's `Tx`.
type RunTx = Rc<dyn Fn(&mut App, &mut Tx)>;

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

/// A footer action on a failed-deletion dialog: its label and what it does after closing.
#[derive(Clone)]
pub(crate) struct ErrorAction {
    label: String,
    run: RunTx,
}

impl ErrorAction {
    pub(crate) fn new(label: impl Into<String>, run: impl Fn(&mut App, &mut Tx) + 'static) -> Self {
        Self {
            label: label.into(),
            run: Rc::new(run),
        }
    }
}

#[derive(Clone)]
enum Buttons {
    Close,
    Offer(Offer),
    DeleteFailure {
        retry: Option<ErrorAction>,
        reveal: Option<ErrorAction>,
    },
}

/// One message dialog to show.
#[derive(Clone)]
pub(crate) struct Error {
    title: String,
    summary: Option<String>,
    message: String,
    icon: IconName,
    tone: Tone,
    buttons: Buttons,
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
            buttons: Buttons::Close,
            returns_to: None,
        }
    }

    pub(crate) fn partial_failure(message: impl Into<String>) -> Self {
        Self {
            title: tr("Completed with errors"),
            summary: Some(tr("Some items could not be processed")),
            message: message.into(),
            icon: IconName::Close,
            tone: Tone::Danger,
            buttons: Buttons::Close,
            returns_to: None,
        }
    }

    /// A failed-deletion outcome with optional permanent-delete retry and blocker reveal actions.
    pub(crate) fn delete_failure(
        detail: impl Into<String>,
        retry: Option<ErrorAction>,
        reveal: Option<ErrorAction>,
    ) -> Self {
        let mut error = Self::partial_failure(detail);
        if retry.is_some() || reveal.is_some() {
            error.buttons = Buttons::DeleteFailure { retry, reveal };
        }
        error
    }

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
            buttons: Buttons::Close,
            returns_to: None,
        }
    }

    /// The same error, offering `offer` beside Close.
    pub(crate) fn offering(mut self, offer: Offer) -> Self {
        self.buttons = Buttons::Offer(offer);
        self
    }

    pub(crate) fn returning_to(mut self, key: Option<&'static str>) -> Self {
        self.returns_to = key;
        self
    }
}

/// The dialog's own texts, from the catalogs once.
struct Labels {
    summary: String,
    close: String,
    done: String,
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

fn dismiss(app: &mut App) -> bool {
    std::mem::replace(&mut app.error_dialog.open, false)
}

fn close(app: &mut App) {
    dismiss(app);
}

/// The offered action: the dialog closes, then the action runs between `publish` and `settle`.
fn act(app: &mut App, tx: &mut Tx) {
    if !dismiss(app) {
        return;
    }
    let Some(run) = app
        .error_dialog
        .shown
        .as_ref()
        .and_then(|error| match &error.buttons {
            Buttons::Offer(offer) => Some(offer.run.clone()),
            Buttons::Close | Buttons::DeleteFailure { .. } => None,
        })
    else {
        return;
    };
    shell::publish(app);
    run(app);
    shell::settle(app, tx);
}

fn act_delete_retry(app: &mut App, tx: &mut Tx) {
    run_delete_action(app, tx, |retry, _| retry.as_ref());
}

fn act_delete_reveal(app: &mut App, tx: &mut Tx) {
    run_delete_action(app, tx, |_, reveal| reveal.as_ref());
}

fn run_delete_action(
    app: &mut App,
    tx: &mut Tx,
    pick: for<'a> fn(&'a Option<ErrorAction>, &'a Option<ErrorAction>) -> Option<&'a ErrorAction>,
) {
    if !dismiss(app) {
        return;
    }
    let Some(run) = app
        .error_dialog
        .shown
        .as_ref()
        .and_then(|error| match &error.buttons {
            Buttons::DeleteFailure { retry, reveal } => pick(retry, reveal).map(|a| a.run.clone()),
            Buttons::Close | Buttons::Offer(_) => None,
        })
    else {
        return;
    };
    shell::publish(app);
    run(app, tx);
    shell::settle(app, tx);
}

fn apply_buttons(built: StrataDialog<App>, error: &Error, labels: &Labels) -> StrataDialog<App> {
    match &error.buttons {
        Buttons::Close => built
            .icon(error.icon)
            .tone(error.tone)
            .primary(labels.close.clone(), close),
        Buttons::Offer(offer) => built
            .icon(offer.icon)
            .tone(Tone::Accent)
            .secondary(labels.close.clone(), close)
            .primary_tx(offer.label.clone(), act),
        Buttons::DeleteFailure { retry, reveal } => {
            let built = built.icon(error.icon).tone(error.tone);
            match (retry, reveal) {
                (Some(retry), Some(reveal)) => built
                    .action("done", labels.done.clone(), close)
                    .secondary_tx(reveal.label.clone(), act_delete_reveal)
                    .primary_tx(retry.label.clone(), act_delete_retry),
                (Some(retry), None) => built
                    .secondary(labels.done.clone(), close)
                    .primary_tx(retry.label.clone(), act_delete_retry),
                (None, Some(reveal)) => built
                    .secondary_tx(reveal.label.clone(), act_delete_reveal)
                    .primary(labels.close.clone(), close),
                (None, None) => built.primary(labels.close.clone(), close),
            }
        }
    }
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
    apply_buttons(built, error, labels).build(&app.look, frame.layer);
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
                done: tr("Done"),
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
