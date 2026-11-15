// SPDX-License-Identifier: MIT

//! Paste (`Ctrl+V`, folder menu) and Duplicate (`Ctrl+D`, entry menu), transfer progress in
//! C16's [`DockView`](strata_components::progress_dock::DockView) and
//! [`ProgressDialog`](strata_components::progress_dock::ProgressDialog), and revealing placed
//! items (`src/ui/browser/transfer.rs`, `view_edit.rs`, `events/reveal.rs`).
//!
//! Destination conflict dialogs (`Replace`, `Keep Both`, `Merge`, `Skip`, `Apply to All`) land
//! in A12's conflict-dialog slice; until then, any batch where an item collides stops safely
//! like Cancel, while non-colliding transfers and same-folder numbered copies run end-to-end.

use gio::prelude::*;
use rune::Tx;
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_desktop::keys::{
    Map, Modifiers,
    bindings::{DUPLICATE, PASTE},
};
use strata_fs::{
    adapters::{TransferTargetCollision, gio_file_for_location, transfer_targets},
    app::{Browser, BrowserEvent},
    services::{PasteItem, ShownName, TransferConflict},
};

use crate::{
    App,
    clipboard::{self, paths},
    commands::{Action, Call, Holds, Keys, On, Outcome, Stage},
    error_dialog::{self, Error},
    filter, menus, shell,
    slots::{Heard, Later, Listener, Part, Place, Scope, Setup},
};

mod dock;
mod reveal;
#[cfg(test)]
mod tests;
mod words;

#[derive(Clone, PartialEq, Eq)]
struct PreparingKey {
    destination: Location,
    sources: Vec<Location>,
    move_sources: bool,
}

pub(crate) struct Transfer {
    later: Later<App>,
    ops: dock::Ops,
    preparing: Vec<PreparingKey>,
    pending_reveal: Option<reveal::PendingReveal>,
}

/// Mounts the blocking progress dialog in `Slot::Dialogs` while `Transfer` mounts the
/// bottom-right progress dock in `Slot::Overlays`.
pub(crate) struct TransferDialog;

impl Part for Transfer {
    const PLACE: Place<App> = Place::Overlays(dock::build_overlay);
    const SCOPE: Scope = Scope::Tab;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            ops: dock::Ops::default(),
            preparing: Vec::new(),
            pending_reveal: None,
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, browser, heard| {
            if !queue_transfer_event(event, browser, heard) {
                queue_reveal_event(event, heard);
            }
        }))
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        app.transfer.ops.last_clock = clock;
        app.transfer.ops.dock.set_animations(app.look.animations);
        let cancels = std::mem::take(&mut app.transfer.ops.queued_cancels);
        let cancelled = !cancels.is_empty();
        for op_id in cancels {
            dock::cancel_op(app, op_id);
        }
        if cancelled {
            shell::settle(app, tx);
        }
        app.transfer.ops.dock.tick(clock, tx) || cancelled
    }

    const COMMANDS: &'static [Action<App>] = &[
        Action::new("paste", paste).command(PASTE).keys(&[Keys::at(
            Stage::FileCommands,
            &[PASTE.ignoring(Modifiers::empty())],
        )
        .when(|_, at| !at.text())]),
        Action::new("duplicate", duplicate)
            .command(DUPLICATE)
            .keys(&[
                Keys::at(Stage::DirectCommands, &[DUPLICATE.in_map(Map::Tenxer)])
                    .when(|_, at| !at.text()),
                Keys::at(Stage::FileCommands, &[DUPLICATE.in_map(Map::Default)])
                    .when(|app, at| !at.text() && filter::targets::selected(app).is_none()),
            ]),
    ];
}

impl Part for TransferDialog {
    const PLACE: Place<App> = Place::Dialogs(dock::build_dialog);
    const SCOPE: Scope = Scope::Tab;

    fn new(_: &Setup<'_>) -> Self {
        Self
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.transfer
            .ops
            .dialog_holds_keyboard()
            .then_some(Holds::Modal { field: false })
    }
}

fn queue_transfer_event(
    event: &BrowserEvent,
    browser: &Browser,
    heard: &mut Heard<'_, App>,
) -> bool {
    match event {
        BrowserEvent::TransferStarted { total, moving } => {
            let started = dock::StartedTransfer::capture(browser, *total, *moving);
            heard.then(move |app, tx| {
                dock::on_transfer_started(app, started, tx);
                true
            });
            true
        }
        BrowserEvent::TransferProgress {
            completed_items,
            completed_files,
            total_files,
            current_file,
            transferred_bytes,
            total_bytes,
        } => {
            let update = dock::ProgressUpdate {
                completed_items: *completed_items,
                completed_files: *completed_files,
                total_files: *total_files,
                current_file: current_file.clone(),
                transferred_bytes: *transferred_bytes,
                total_bytes: *total_bytes,
            };
            heard.then(move |app, tx| dock::on_transfer_progress(app, None, update, tx));
            true
        }
        BrowserEvent::FlushingToDevice => {
            heard.then(|app, tx| dock::on_flushing(app, None, tx));
            true
        }
        BrowserEvent::TransferCancellationPending => {
            heard.then(|app, _| dock::on_cancel_pending(app));
            true
        }
        BrowserEvent::BackgroundOperation { request_id, event } => {
            let request_id = *request_id;
            let inner = (**event).clone();
            heard.then(move |app, tx| dock::on_background_event(app, request_id, inner, tx));
            true
        }
        _ => queue_outcome_event(event, heard),
    }
}

fn queue_outcome_event(event: &BrowserEvent, heard: &mut Heard<'_, App>) -> bool {
    match event {
        BrowserEvent::TransferFinished { moved_locations } => {
            let moved = moved_locations.clone();
            heard.then(move |app, _| {
                dock::on_transfer_finished(app, None, &moved);
                true
            });
            true
        }
        BrowserEvent::TransferCompleted => {
            heard.then(|app, tx| dock::on_transfer_completed(app, None, tx));
            true
        }
        BrowserEvent::ItemsReported { report } => {
            let report = report.clone();
            heard.then(move |app, tx| dock::on_items_reported(app, &report, tx));
            true
        }
        BrowserEvent::OperationFailed {
            message, report, ..
        } => {
            let message = message.clone();
            let report = report.clone();
            heard.then(move |app, _| dock::on_operation_failed(app, None, message, &report));
            true
        }
        BrowserEvent::OperationCancelled {
            completed,
            failed,
            not_attempted,
            affected_locations,
            report,
        } => {
            let outcome = dock::CancelledOutcome {
                completed: *completed,
                failed: *failed,
                not_attempted: *not_attempted,
                affected_locations: affected_locations.clone(),
                report: report.clone(),
            };
            heard.then(move |app, _| dock::on_operation_cancelled(app, None, outcome));
            true
        }
        BrowserEvent::OperationRefreshRequired { depths } => {
            let depths = depths.clone();
            heard.then(move |app, _| {
                app.browser.refresh_operation_columns(&depths);
                false
            });
            true
        }
        _ => false,
    }
}

fn queue_reveal_event(event: &BrowserEvent, heard: &mut Heard<'_, App>) {
    match event {
        BrowserEvent::TransferReveal {
            destination,
            locations,
        } => {
            let destination = destination.clone();
            let locations = locations.clone();
            heard.then(move |app, tx| {
                reveal::apply(app, destination, locations, tx);
                true
            });
        }
        BrowserEvent::EntriesInserted { depth, .. }
        | BrowserEvent::EntriesReplaced { depth, .. }
        | BrowserEvent::EntriesPublished { depth, .. }
        | BrowserEvent::EntriesSpliced { depth, .. }
        | BrowserEvent::LoadFinished { depth, .. }
        | BrowserEvent::ColumnReloaded { depth } => {
            let depth = *depth;
            heard.then(move |app, _| {
                reveal::on_listing_changed(app, depth);
                false
            });
        }
        BrowserEvent::Reset => {
            heard.then(|app, _| {
                app.transfer.pending_reveal = None;
                false
            });
        }
        _ => {}
    }
}

fn valid_paste_destination(location: &Location) -> bool {
    !paths::is_trash(location) && !location.is_recent_location()
}

fn shows_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

pub(crate) fn paste_destination(
    selected: &[FileEntry],
    column: Option<Location>,
    load_cursor: bool,
) -> Option<Location> {
    let target = match selected {
        [folder] if folder.is_directory() && !load_cursor => Some(folder.location.clone()),
        _ => column,
    };
    target.filter(valid_paste_destination)
}

fn resolve_paste_location(app: &App, on: &On) -> Option<Location> {
    match on {
        On::Column(depth) => app
            .browser
            .location_at(*depth)
            .filter(valid_paste_destination),
        On::Place(location) => Some(location.clone()).filter(valid_paste_destination),
        On::Row((depth, _)) => app
            .browser
            .location_at(*depth)
            .filter(valid_paste_destination),
        On::Focus => {
            let selected = if app.browser.selected_count() == 1 {
                app.browser.selected_entries()
            } else {
                Vec::new()
            };
            let depth = if shows_columns(app) {
                Some(app.columns.targeted())
            } else {
                app.browser.active_depth()
            };
            let column = depth
                .and_then(|d| app.browser.location_at(d))
                .or_else(|| app.browser.active_location());
            paste_destination(&selected, column, app.browser.selection_is_load_cursor())
        }
    }
}

fn paste(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    let Some(destination) = resolve_paste_location(app, &call.on) else {
        return Outcome::Ran;
    };
    let later = app.transfer.later.clone();
    clipboard::read_files(
        app,
        move |app, decoded, tx| {
            let Some((sources, foreign_cut)) = decoded else {
                return;
            };
            later.then(move |app, _| {
                let move_sources = foreign_cut || clipboard::is_cut_match(app, &sources);
                begin_transfer(app, destination, sources, move_sources);
                false
            });
            shell::settle(app, tx);
        },
        tx,
    );
    Outcome::Ran
}

fn focus_duplicate_entries(app: &App) -> Vec<FileEntry> {
    let tenxer = shell::preferences(app).tenxer_mode;
    if let Some(results) = filter::targets::selected(app) {
        if !results.is_empty() || !tenxer {
            return results;
        }
        return filter::targets::under_cursor(app)
            .map(|(_, entry)| entry)
            .into_iter()
            .collect();
    }
    if !tenxer {
        return app.browser.transfer_entries();
    }
    let depth = if shows_columns(app) {
        Some(app.columns.targeted())
    } else {
        app.browser.active_depth()
    };
    depth
        .map(|d| app.browser.command_entries(d))
        .unwrap_or_default()
}

fn duplicate_entries(app: &App, on: &On) -> Vec<FileEntry> {
    match *on {
        On::Row((depth, id)) => menus::context_entries(app, depth, id)
            .map(|(_, entries)| entries)
            .unwrap_or_default(),
        On::Focus => focus_duplicate_entries(app),
        On::Column(_) | On::Place(_) => Vec::new(),
    }
}

fn duplicate_transfer(entries: &[FileEntry]) -> Option<(Location, Vec<Location>)> {
    let destination = entries.first()?.location.parent()?;
    let shares_parent = entries
        .iter()
        .all(|entry| entry.location.parent().as_ref() == Some(&destination));
    if paths::is_trash(&destination) || !shares_parent {
        return None;
    }
    let sources = entries.iter().map(|entry| entry.location.clone()).collect();
    Some((destination, sources))
}

fn duplicate(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let entries = duplicate_entries(app, &call.on);
    let Some((destination, sources)) = duplicate_transfer(&entries) else {
        return match call.on {
            On::Focus => Outcome::Declined,
            _ => Outcome::Ran,
        };
    };
    begin_transfer(app, destination, sources, false);
    Outcome::Ran
}

fn transfer_is_noop(source: &Location, destination: &Location, move_sources: bool) -> bool {
    let source_file = gio_file_for_location(source);
    let dest_file = gio_file_for_location(destination);
    let moves_into_own_parent = || {
        source_file
            .parent()
            .is_some_and(|parent| parent.equal(&dest_file))
    };
    source_file.equal(&dest_file)
        || dest_file.has_prefix(&source_file)
        || (move_sources && moves_into_own_parent())
}

fn transfer_refused(destination: &Location, sources: &[Location], move_sources: bool) -> bool {
    !valid_paste_destination(destination)
        || (move_sources && sources.iter().any(|source| !paths::can_remove(source)))
}

type TargetTuple = (
    Location,
    Option<ShownName>,
    Option<TransferTargetCollision>,
    bool,
);

fn begin_transfer(
    app: &mut App,
    destination: Location,
    sources: Vec<Location>,
    move_sources: bool,
) {
    if transfer_refused(&destination, &sources, move_sources) {
        return;
    }
    let sources: Vec<Location> = sources
        .into_iter()
        .filter(|source| !transfer_is_noop(source, &destination, move_sources))
        .collect();
    if sources.is_empty() {
        return;
    }
    let key = PreparingKey {
        destination: destination.clone(),
        sources: sources.clone(),
        move_sources,
    };
    if app.transfer.preparing.contains(&key) {
        return;
    }
    app.transfer.preparing.push(key.clone());
    let later = app.transfer.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let cancellable = gio::Cancellable::new();
        let result = transfer_targets(&destination, &sources, move_sources, &cancellable)
            .await
            .map(|targets| {
                sources
                    .into_iter()
                    .zip(targets)
                    .map(|(source, target)| {
                        (source, target.name, target.collision, target.fixes_inside)
                    })
                    .collect::<Vec<TargetTuple>>()
            })
            .map_err(|error| error.message().to_owned());
        later.then(move |app, tx| {
            finish_preparing(app, &key, result, tx);
            true
        });
    });
}

fn finish_preparing(
    app: &mut App,
    key: &PreparingKey,
    result: Result<Vec<TargetTuple>, String>,
    tx: &mut Tx,
) {
    app.transfer.preparing.retain(|active| active != key);
    let targets = match result {
        Ok(targets) => targets,
        Err(message) => {
            error_dialog::show(app, Error::new(tr("Unable to transfer"), message));
            return;
        }
    };
    if targets
        .iter()
        .any(|(_, _, collision, _)| collision.is_some())
    {
        return;
    }
    let accepted: Vec<PasteItem> = targets
        .into_iter()
        .map(|(source, target_name, _, _)| PasteItem {
            source,
            conflict: TransferConflict::FailIfExists,
            target_name,
        })
        .collect();
    if accepted.is_empty() {
        return;
    }
    shell::publish(app);
    app.browser
        .transfer(key.destination.clone(), accepted, key.move_sources, true);
    shell::settle(app, tx);
}
