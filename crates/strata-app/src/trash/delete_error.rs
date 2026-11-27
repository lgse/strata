// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_fs::services::DeleteBlocker;

use super::{
    dialog::Kind,
    sizes::{Reach, deletion_reach},
};
use crate::{
    App,
    error_dialog::{self, Error, ErrorAction},
    filter, shell,
};

/// An operation that finished with errors (`OperationCompletedWithErrors`).
pub(crate) struct Failed {
    pub(crate) message: String,
    pub(crate) retryable: Vec<Location>,
    pub(crate) others_failed: bool,
    pub(crate) blocker: Option<DeleteBlocker>,
}

pub(super) struct DeleteErrorRetry {
    pub(super) entries: Vec<FileEntry>,
    pub(super) require_choice: bool,
}

pub(super) fn completed_with_errors(app: &mut App, failed: Failed) -> bool {
    let entries =
        retryable_delete_entries(std::mem::take(&mut app.trash.trashing), &failed.retryable);
    show_partial_operation_failure(
        app,
        failed.message,
        failed.blocker,
        DeleteErrorRetry {
            entries,
            require_choice: failed.others_failed,
        },
    )
}

pub(crate) fn background_completed_with_errors(app: &mut App, failed: Failed) -> bool {
    let mut entries = Vec::new();
    app.trash.trashing.retain(|entry| {
        if failed.retryable.contains(&entry.location) {
            entries.push(entry.clone());
            false
        } else {
            true
        }
    });
    let require_choice = failed.others_failed || app.browser.has_foreground_operation();
    show_partial_operation_failure(
        app,
        failed.message,
        failed.blocker,
        DeleteErrorRetry {
            entries,
            require_choice,
        },
    )
}

pub(super) fn show_partial_operation_failure(
    app: &mut App,
    message: String,
    blocker: Option<DeleteBlocker>,
    retry: DeleteErrorRetry,
) -> bool {
    if !retry.entries.is_empty() && !retry.require_choice {
        super::show(app, retry.entries, Kind::TrashUnavailable);
        return true;
    }
    let reveal = blocker.map(delete_reveal_action);
    let retry = (!retry.entries.is_empty()).then(|| delete_retry_action(retry.entries));
    if reveal.is_none() && retry.is_none() {
        error_dialog::show(app, Error::partial_failure(message));
    } else {
        error_dialog::show(app, Error::delete_failure(message, retry, reveal));
    }
    true
}

fn retryable_delete_entries(
    entries: Vec<FileEntry>,
    retryable_locations: &[Location],
) -> Vec<FileEntry> {
    entries
        .into_iter()
        .filter(|entry| retryable_locations.contains(&entry.location))
        .collect()
}

fn delete_retry_label(entries: &[FileEntry]) -> String {
    tr(match deletion_reach(entries) {
        Reach::Local => "Delete Permanently",
        Reach::Remote => "Delete without Trash",
    })
}

fn delete_retry_action(entries: Vec<FileEntry>) -> ErrorAction {
    ErrorAction::new(delete_retry_label(&entries), move |app: &mut App, _| {
        super::show(app, entries.clone(), Kind::TrashUnavailable);
    })
}

fn delete_reveal_action(blocker: DeleteBlocker) -> ErrorAction {
    ErrorAction::new(reveal_label(&blocker), move |app: &mut App, _| {
        reveal_delete_blocker(app, blocker.clone());
    })
}

fn reveal_delete_blocker(app: &App, blocker: DeleteBlocker) {
    let later = app.trash.later.clone();
    let generation = app.browser.navigation_generation();
    glib::MainContext::default().spawn_local(async move {
        let target = blocker.reveal_target().await;
        later.then(move |app: &mut App, tx: &mut Tx| {
            apply_reveal_target(app, generation, target, tx)
        });
    });
}

fn apply_reveal_target(
    app: &mut App,
    generation: u64,
    target: Option<(Location, Location)>,
    tx: &mut Tx,
) -> bool {
    if app.browser.navigation_generation() != generation {
        return false;
    }
    match target {
        Some((folder, item)) => {
            if let Some(depth) = app.browser.open_depth(&folder) {
                filter::clear_for_reveal(app, depth, tx);
            }
            app.browser.reveal_locations(folder, vec![item]);
            if let Some(view) = shell::view_focus(app) {
                tx.focus(view, false);
            }
        }
        None => error_dialog::show(
            app,
            Error::new(
                tr("Unable to open location"),
                tr("The item's location could not be verified."),
            ),
        ),
    }
    true
}

fn reveal_label(blocker: &DeleteBlocker) -> String {
    match blocker.child.file_name() {
        Some(name) if blocker.one_of_several => {
            rust_i18n::t!("Show “%{name}” in Folder", name = name.to_string_lossy()).into_owned()
        }
        _ => tr("Show in Folder"),
    }
}

#[cfg(test)]
mod tests;
