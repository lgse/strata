// SPDX-License-Identifier: MIT

//! Empty Trash, as GTK's `confirm_empty_trash` runs it (`load_trash_summary` and
//! `show_empty_trash_confirmation` in `src/ui/browser/trash/empty.rs`): the command
//! `empty-trash`, which the sidebar's Trash menu runs.
//!
//! - **Measuring.** Unless the view lists the Trash and shows it empty, a dialog says
//!   "Measuring Trash…" with a spinner while the Trash is walked (`summarize_directory`); its
//!   Cancel, Escape and close button stop the walk. A Trash found empty closes it; a walk that
//!   fails says "Unable to read Trash".
//! - **The confirmation.** "Empty Trash?", what will be reclaimed, and that it cannot be
//!   undone, in the danger tone, with Cancel and Empty Trash. Empty Trash has the keyboard, as
//!   GTK focuses it, and Return presses only the focused button.
//! - **Emptying** starts only from that dialog's Empty Trash, while it is open and showing the
//!   confirmation the press came from: each dialog carries its own generation, and a press on a
//!   dialog that has closed or been replaced does nothing. The Trash columns are read again
//!   after; items that could not be deleted, and a failure, are reported in the error dialog.
//! - **One at a time.** While any of the part's dialogs is open, the command does nothing: an
//!   open dialog is never replaced under the keyboard.
//!
//! Left out: GTK's progress card while the Trash empties ("Emptying Trash", the count deleted
//! and Cancel), which whoever mounts the file operations' progress (C16) shows; and the
//! partial failure's own dialog look ("Completed with errors" is shown as an error).

use std::time::Instant;

use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::{
    i18n::{count, file_size, tr},
    model::Location,
};
use strata_fs::adapters::{
    directory_summary::{DirectorySummary, summarize_directory},
    gio_file_for_location,
    trash::{EmptyTrashOutcome, empty_trash},
};
use strata_kit::icons::IconName;

use super::sizes::item_count_label;
use crate::{
    App,
    commands::{Call, Outcome},
    error_dialog::{self, Error},
    shell,
    slots::{Frame, Later},
};

/// Where the Trash is: `trash:///`. A hosted test has no gvfs to list it with, so a test
/// points it at a folder of its own; nothing else can.
pub(crate) fn root() -> Location {
    #[cfg(test)]
    if let Some(folder) = TEST_ROOT.with(|root| root.borrow().clone()) {
        return Location::local(folder);
    }
    Location::uri("trash:///")
}

#[cfg(test)]
thread_local! {
    static TEST_ROOT: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Points [`root`] at `folder` for this test.
#[cfg(test)]
pub(crate) fn use_test_root(folder: &std::path::Path) {
    TEST_ROOT.with(|root| root.replace(Some(folder.to_owned())));
}

/// What the dialog shows.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Measuring,
    Confirm { subtitle: String },
}

/// One Empty Trash dialog: what it shows, and its generation, which a walk's answer and a
/// confirmation are matched against.
pub(super) struct Dialog {
    phase: Phase,
    generation: u64,
}

/// The part's share of Empty Trash.
#[derive(Default)]
pub(super) struct Empty {
    /// The dialog, kept after it closes for its closing motion.
    dialog: Option<Dialog>,
    pub(super) open: bool,
    /// The walk of the Trash, which stops when the dialog closes.
    measuring: Option<glib::JoinHandle<()>>,
    generations: u64,
    /// The generation whose Empty Trash was pressed, for the part's next `poll`.
    pub(super) confirmed: Option<u64>,
}

/// The command: GTK's `confirm_empty_trash`.
pub(super) fn run(app: &mut App, _: &Call<'_>, _: &mut rune::Tx) -> Outcome {
    if app.trash.open || app.trash.empty.open || listed_empty(app) {
        return Outcome::Ran;
    }
    let empty = &mut app.trash.empty;
    empty.generations += 1;
    let generation = empty.generations;
    empty.dialog = Some(Dialog {
        phase: Phase::Measuring,
        generation,
    });
    empty.open = true;
    let task = measure(&app.trash.later, generation);
    if let Some(earlier) = app.trash.empty.measuring.replace(task) {
        earlier.abort();
    }
    Outcome::Ran
}

/// `trash_listing_is_empty`: the view lists the Trash, and it shows nothing.
fn listed_empty(app: &App) -> bool {
    let trash = root();
    let depths = (0..).map_while(|depth| Some((depth, app.browser.location_at(depth)?)));
    depths
        .filter(|(_, location)| *location == trash)
        .any(|(depth, _)| {
            app.browser
                .column_entry_counts(depth)
                .is_some_and(|counts| counts.total == 0)
        })
}

/// `load_trash_summary`'s walk, answering through `later`.
fn measure(later: &Later<App>, generation: u64) -> glib::JoinHandle<()> {
    let later = later.clone();
    glib::MainContext::default().spawn_local(async move {
        let started = Instant::now();
        let result = summarize_directory(&gio_file_for_location(&root())).await;
        let measured = Measured::of(result, started);
        later.then(move |app: &mut App, _| measured.apply(app, generation));
    })
}

/// What the walk found, as the dialog takes it (`finish_trash_summary`).
enum Measured {
    Items(String),
    Nothing,
    Failed(String),
}

impl Measured {
    fn of(result: Result<DirectorySummary, glib::Error>, started: Instant) -> Self {
        match result {
            Ok(summary) if summary.item_count > 0 => {
                tracing::info!(
                    item_count = summary.item_count,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    truncated = summary.truncated(),
                    "trash summary built"
                );
                Self::Items(subtitle(&summary))
            }
            Ok(_) => Self::Nothing,
            Err(error) => {
                tracing::warn!(%error, "trash summary failed");
                Self::Failed(error.to_string())
            }
        }
    }

    /// Takes the answer while the dialog it was asked for is open and still measuring.
    fn apply(self, app: &mut App, generation: u64) -> bool {
        let empty = &mut app.trash.empty;
        let measuring = empty.open
            && empty.dialog.as_ref().is_some_and(|dialog| {
                dialog.generation == generation && dialog.phase == Phase::Measuring
            });
        if !measuring {
            return false;
        }
        empty.measuring = None;
        match self {
            Self::Items(subtitle) => {
                empty.generations += 1;
                empty.dialog = Some(Dialog {
                    phase: Phase::Confirm { subtitle },
                    generation: empty.generations,
                });
            }
            Self::Nothing => empty.open = false,
            Self::Failed(message) => {
                empty.open = false;
                error_dialog::show(app, Error::new(tr("Unable to read Trash"), message));
            }
        }
        true
    }
}

/// `empty_trash_subtitle`.
fn subtitle(summary: &DirectorySummary) -> String {
    let message = if summary.truncated() {
        "At least %{items} · at least %{size} will be reclaimed"
    } else {
        "%{items} · %{size} will be reclaimed"
    };
    rust_i18n::t!(
        message,
        items = item_count_label(summary.item_count),
        size = file_size(summary.total_size)
    )
    .into_owned()
}

/// Closes the dialog and stops its walk: Cancel, Escape, the close button, a click outside.
fn cancelled(app: &mut App) {
    let empty = &mut app.trash.empty;
    empty.open = false;
    if let Some(task) = empty.measuring.take() {
        task.abort();
    }
}

/// Empty Trash on the confirmation: only while that dialog is open and confirming.
pub(super) fn confirmed(app: &mut App) {
    let empty = &mut app.trash.empty;
    let Some(dialog) = empty.dialog.as_ref().filter(|_| empty.open) else {
        return;
    };
    if !matches!(dialog.phase, Phase::Confirm { .. }) {
        return;
    }
    empty.confirmed = Some(dialog.generation);
    empty.open = false;
}

/// The part's `poll`: empties the Trash for the confirmation pressed, if it is still the
/// last one shown.
pub(super) fn poll(app: &mut App) -> bool {
    let Some(generation) = app.trash.empty.confirmed.take() else {
        return false;
    };
    let latest = app
        .trash
        .empty
        .dialog
        .as_ref()
        .map(|dialog| dialog.generation);
    if latest != Some(generation) {
        return false;
    }
    app.browser.focus_active();
    let later = app.trash.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let trash = root();
        let result = empty_trash(&gio_file_for_location(&trash), |_| {}).await;
        later.then(move |app: &mut App, _| emptied(app, &trash, result));
    });
    true
}

/// `report_empty_trash_result`.
fn emptied(
    app: &mut App,
    trash: &Location,
    result: Result<EmptyTrashOutcome, glib::Error>,
) -> bool {
    match result {
        Ok(outcome) => {
            app.browser.refresh_columns_at(trash);
            if outcome.failed > 0 {
                let error = Error::new(tr("Completed with errors"), failure_summary(&outcome));
                error_dialog::show(app, error);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "empty trash failed");
            error_dialog::show(
                app,
                Error::new(tr("Unable to empty Trash"), error.to_string()),
            );
        }
    }
    true
}

/// `empty_trash_error_summary`.
fn failure_summary(outcome: &EmptyTrashOutcome) -> String {
    let mut summary = rust_i18n::t!(
        "%{items} could not be deleted. The remaining items were processed.",
        items = item_count_label(outcome.failed)
    )
    .into_owned();
    for error in &outcome.errors {
        summary.push_str("\n\n• ");
        summary.push_str(error);
    }
    if outcome.failed > outcome.errors.len() {
        summary.push_str("\n\n");
        summary.push_str(&count("more_items", outcome.failed - outcome.errors.len()));
    }
    summary
}

/// The dialog, while it is open or closing.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) {
    let empty = &app.trash.empty;
    let Some(dialog) = empty.dialog.as_ref() else {
        return;
    };
    let key = format!("empty-trash-{}", dialog.generation);
    let built = match &dialog.phase {
        Phase::Measuring => {
            let title = tr("Measuring Trash…");
            StrataDialog::new(key, title.clone(), cancelled)
                .loading(Some(title))
                .message(tr(
                    "Calculating the number and size of items. This may take a few seconds.",
                ))
                .secondary(tr("Cancel"), cancelled)
                .initial_focus(Focus::Secondary)
        }
        Phase::Confirm { subtitle } => StrataDialog::new(key, tr("Empty Trash?"), cancelled)
            .subtitle(subtitle.clone())
            .tone(Tone::Danger)
            .message(tr(
                "Everything in Trash will be permanently deleted. This action cannot be undone.",
            ))
            .secondary(tr("Cancel"), cancelled)
            .primary(tr("Empty Trash"), confirmed)
            .initial_focus(Focus::Primary),
    };
    let mut built = built
        .open(empty.open)
        .icon(IconName::Trash)
        // Return presses the focused button only: nothing is the default.
        .default_button(false)
        .close_label(tr("Close dialog"))
        .width(MESSAGE_WIDTH);
    if let Some(view) = shell::view_focus(app) {
        built = built.return_focus(view);
    }
    built.build(&app.look, frame.layer);
}

/// The dialog is open: the window's commands wait.
pub(super) fn holds(app: &App) -> bool {
    app.trash.empty.open
}

#[cfg(test)]
pub(super) fn shown(app: &App) -> Option<(bool, &'static str)> {
    let dialog = app.trash.empty.dialog.as_ref()?;
    let phase = match dialog.phase {
        Phase::Measuring => "measuring",
        Phase::Confirm { .. } => "confirm",
    };
    Some((app.trash.empty.open, phase))
}
