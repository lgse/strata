// SPDX-License-Identifier: MIT

mod batch;
#[cfg(test)]
mod tests;
mod view;

use rune::{TextEdit, Tx};
use strata_core::{
    i18n::tr,
    model::{ArchiveFormat, FileEntry, Location},
};
use strata_fs::{
    app::BrowserEvent,
    services::{PasswordFailure, TransferReport},
};

use crate::{
    App,
    commands::{Action, Call, Holds, On, Outcome},
    error_dialog::{self, Error},
    menus, shell,
    slots::{Heard, Listener, Part, Place, Scope, Setup},
};

struct PasswordTarget {
    entry: FileEntry,
    destination: Location,
    navigate_after_extract: Option<Location>,
}

struct PasswordForm {
    entry: FileEntry,
    destination: Location,
    navigate_after_extract: Option<Location>,
    password: TextEdit,
    revealed: bool,
    error: Option<&'static str>,
    generation: u64,
    attempt: u64,
}

impl PasswordForm {
    fn new(target: PasswordTarget, invalid_password: bool, generation: u64) -> Self {
        let mut password = TextEdit::new("").secure();
        password.focused = true;
        Self {
            entry: target.entry,
            destination: target.destination,
            navigate_after_extract: target.navigate_after_extract,
            password,
            revealed: false,
            error: invalid_password.then_some("Invalid password"),
            generation,
            attempt: u64::from(invalid_password),
        }
    }

    fn wipe_secret(&mut self) {
        self.password = TextEdit::new("").secure();
        self.revealed = false;
    }
}

pub(crate) struct Archive {
    form: Option<PasswordForm>,
    open: bool,
    opened: u64,
    pending_destination: Option<Location>,
    pending_select: Vec<String>,
    extract_destination: Option<Location>,
    pending_extract_retry: Option<(FileEntry, Location)>,
    pending_navigate: Option<Location>,
    batch: Option<batch::Batch>,
}

/// Why a selection is not extracted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    Nothing,
    NotArchives,
}

impl Part for Archive {
    const PLACE: Place<App> = Place::Dialogs(view::build);
    const SCOPE: Scope = Scope::Tab;

    fn new(_: &Setup<'_>) -> Self {
        Self {
            form: None,
            open: false,
            opened: 0,
            pending_destination: None,
            pending_select: Vec::new(),
            extract_destination: None,
            pending_extract_retry: None,
            pending_navigate: None,
            batch: None,
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            queue_event(event, heard);
        }))
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let form = app.archive.form.as_ref().filter(|_| app.archive.open)?;
        Some(Holds::Modal {
            field: form.password.focused,
        })
    }

    const COMMANDS: &'static [Action<App>] = &[
        Action::new("extract", extract_here),
        Action::new("extract-to", crate::chooser::floating::extract_to),
    ];
}

fn shows_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

/// Error text may contain arbitrary filenames; only the structured kind permits retry.
fn extract_password_retry(password_failure: Option<PasswordFailure>) -> Option<bool> {
    password_failure.map(|failure| failure == PasswordFailure::Incorrect)
}

fn record_password_retry(archive: &mut Archive, entry: &FileEntry, destination: &Location) {
    let supports = ArchiveFormat::from_extension(&entry.display_name)
        .is_some_and(|format| format.supports_password());
    archive.pending_extract_retry = supports.then(|| (entry.clone(), destination.clone()));
}

fn extract_here(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    let entries = match call.on {
        On::Row((depth, id)) => menus::context_entries(app, depth, id).map(|(_, entries)| entries),
        On::Focus => {
            shell::publish(app);
            Some(app.browser.selected_entries())
        }
        On::Column(_) | On::Place(_) => None,
    };
    if let Some(entries) = entries.filter(|entries| extractable(entries).is_ok()) {
        extract_entries(app, entries, tx);
    }
    Outcome::Ran
}

/// Whether "Extract here" takes `entries`: every one a regular local archive, or none run.
pub(crate) fn extractable(entries: &[FileEntry]) -> Result<(), Refused> {
    if entries.is_empty() {
        return Err(Refused::Nothing);
    }
    if entries
        .iter()
        .all(|entry| ArchiveFormat::for_entry(entry).is_some())
    {
        Ok(())
    } else {
        Err(Refused::NotArchives)
    }
}

/// Extracts each archive beside itself, one after another.
pub(crate) fn extract_entries(app: &mut App, mut entries: Vec<FileEntry>, tx: &mut Tx) {
    if entries.len() > 1 {
        batch::start(app, entries, tx);
    } else if let Some(entry) = entries.pop() {
        extract_entry(app, entry, tx);
    }
}

pub(crate) fn extract_entry(app: &mut App, entry: FileEntry, tx: &mut Tx) {
    if entry.location.native_path().is_none() {
        return;
    }
    let Some(parent) = entry.location.parent() else {
        error_dialog::show(
            app,
            Error::new(
                tr("Cannot extract"),
                tr("This archive has no parent directory."),
            ),
        );
        return;
    };
    extract_entry_to(app, entry, parent, tx);
}

pub(crate) fn extract_entry_to(
    app: &mut App,
    entry: FileEntry,
    destination: Location,
    tx: &mut Tx,
) {
    app.archive.batch = None;
    begin_extract(app, entry, destination, tx);
}

fn begin_extract(app: &mut App, entry: FileEntry, destination: Location, tx: &mut Tx) {
    app.archive.extract_destination = Some(destination.clone());
    record_password_retry(&mut app.archive, &entry, &destination);
    shell::publish(app);
    app.browser.extract(entry, destination, false, None);
    shell::settle(app, tx);
}

pub(crate) fn extract_to_chosen(
    app: &mut App,
    entry: FileEntry,
    destination: Location,
    tx: &mut Tx,
) {
    app.archive.pending_navigate = Some(destination.clone());
    extract_entry_to(app, entry, destination, tx);
}

pub(crate) fn clear_extract_destination(app: &mut App) {
    app.archive.extract_destination = None;
}

pub(crate) fn prepare_compression(app: &mut App, destination: Location) {
    app.archive.extract_destination = None;
    app.archive.pending_destination = Some(destination);
    app.archive.pending_select.clear();
}

pub(crate) fn take_pending_destination(app: &mut App) -> Option<Location> {
    app.archive.pending_navigate = None;
    app.archive.pending_destination.take()
}

/// Drops what a cancelled operation left pending, returning what its batch had still to report.
pub(crate) fn abandon_follow_ups(app: &mut App) -> Option<String> {
    let archive = &mut app.archive;
    archive.pending_destination = None;
    archive.pending_select.clear();
    archive.extract_destination = None;
    archive.pending_extract_retry = None;
    archive.pending_navigate = None;
    archive.batch.take().and_then(|batch| batch.detail())
}

/// The archive being extracted and how many "Extract here" took together.
pub(crate) fn batch_position(app: &App) -> Option<(usize, usize)> {
    app.archive.batch.as_ref().map(batch::Batch::position)
}

/// Whether a batch has an archive extracting, so its progress stays up between archives.
pub(crate) fn batch_extracting(app: &App) -> bool {
    app.archive
        .batch
        .as_ref()
        .is_some_and(batch::Batch::extracting)
}

/// Keeps a batch archive's report for the batch's end; `false` outside a batch.
pub(crate) fn defer_report(app: &mut App, report: &TransferReport) -> bool {
    match &mut app.archive.batch {
        Some(batch) => {
            batch.absorb(report.clone());
            true
        }
        None => false,
    }
}

/// What a failed foreground operation still needs: `detail` is shown when this returns `false`.
pub(crate) fn on_operation_failed(
    app: &mut App,
    password_failure: Option<PasswordFailure>,
    detail: &str,
    tx: &mut Tx,
) -> bool {
    let archive = &mut app.archive;
    archive.pending_destination = None;
    archive.pending_select.clear();
    archive.extract_destination = None;
    let retry = archive.pending_extract_retry.take();
    let navigate_after_extract = archive.pending_navigate.take();
    let prompt = retry.zip(extract_password_retry(password_failure));
    let Some(((entry, destination), invalid_password)) = prompt else {
        return batch::on_archive_failed(app, detail, tx);
    };
    if let Some(batch) = &mut app.archive.batch {
        batch.pause();
    }
    open_password_dialog(
        app,
        PasswordTarget {
            entry,
            destination,
            navigate_after_extract,
        },
        invalid_password,
        tx,
    );
    true
}

fn open_password_dialog(
    app: &mut App,
    target: PasswordTarget,
    invalid_password: bool,
    tx: &mut Tx,
) {
    let archive = &mut app.archive;
    if let Some(previous) = &mut archive.form {
        previous.wipe_secret();
    }
    archive.opened += 1;
    let generation = archive.opened;
    archive.form = Some(PasswordForm::new(target, invalid_password, generation));
    archive.open = true;
    tx.focus_when_shown(view::password_key(generation), false);
}

/// Closing the prompt gives up on its archive, and on the batch's archives not yet started.
fn cancel(app: &mut App, tx: &mut Tx) {
    app.archive.open = false;
    if let Some(form) = &mut app.archive.form {
        form.wipe_secret();
    }
    batch::finish(app, tx);
}

fn clear_error(app: &mut App) {
    if let Some(form) = &mut app.archive.form {
        form.error = None;
    }
}

fn toggle_reveal(app: &mut App) {
    if let Some(form) = &mut app.archive.form {
        form.revealed = !form.revealed;
    }
}

fn submit(app: &mut App, tx: &mut Tx) {
    if !app.archive.open {
        return;
    }
    let Some(form) = app.archive.form.as_mut() else {
        return;
    };
    if form.password.text.is_empty() {
        form.error = Some("Enter a password");
        form.attempt += 1;
        form.password.focused = true;
        tx.focus(view::password_key(form.generation), false);
        return;
    }
    let entry = form.entry.clone();
    let destination = form.destination.clone();
    let navigate = form.navigate_after_extract.clone();
    let password = form.password.text.clone();
    form.wipe_secret();
    app.archive.open = false;
    app.archive.extract_destination = Some(destination.clone());
    app.archive.pending_navigate = navigate;
    if let Some(batch) = &mut app.archive.batch {
        batch.resume();
    }
    record_password_retry(&mut app.archive, &entry, &destination);
    shell::publish(app);
    app.browser
        .extract(entry, destination, false, Some(password));
    shell::settle(app, tx);
}

fn queue_event(event: &BrowserEvent, heard: &mut Heard<'_, App>) {
    match event {
        BrowserEvent::ExtractRequested { entry } => {
            let entry = entry.clone();
            heard.then(move |app, tx| {
                extract_entry(app, entry, tx);
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
            heard.then(move |app, tx| reveal_pending_at(app, depth, tx));
        }
        BrowserEvent::OperationCompletedWithErrors { .. } => {
            heard.then(|app, _| {
                app.archive.pending_destination = None;
                app.archive.pending_select.clear();
                false
            });
        }
        BrowserEvent::Reset => {
            heard.then(|app, _| {
                if app.archive.pending_destination.as_ref() != app.browser.location_at(0).as_ref() {
                    app.archive.pending_destination = None;
                    app.archive.pending_select.clear();
                }
                false
            });
        }
        _ => {}
    }
}

pub(crate) fn on_completed(
    app: &mut App,
    select_name: String,
    cancelled: bool,
    tx: &mut Tx,
) -> bool {
    app.archive.pending_extract_retry = None;
    let extract_destination = app.archive.extract_destination.take();
    if app.archive.batch.is_some() {
        app.archive.pending_destination = None;
        app.archive.pending_select.clear();
        let extracted = extract_destination.filter(|_| !select_name.is_empty());
        return batch::on_archive_done(app, extracted.zip(Some(select_name)), cancelled, tx);
    }
    if select_name.is_empty() {
        app.archive.pending_destination = None;
        app.archive.pending_select.clear();
    }
    if let Some(destination) = app.archive.pending_navigate.take() {
        return navigate_to_result(app, destination, select_name, tx);
    }
    let pending_destination = (!select_name.is_empty())
        .then(|| app.archive.pending_destination.clone())
        .flatten();
    match pending_destination {
        Some(destination) => reveal_compressed(app, destination, select_name, tx),
        None if !select_name.is_empty() => {
            show_archive_result(app, extract_destination, vec![select_name], tx)
        }
        None => false,
    }
}

pub(crate) fn on_background_completed(
    app: &mut App,
    destination: Option<Location>,
    select_name: String,
    tx: &mut Tx,
) -> bool {
    if select_name.is_empty() {
        return false;
    }
    let Some(destination) = destination else {
        return false;
    };
    app.archive.pending_destination = Some(destination.clone());
    reveal_compressed(app, destination, select_name, tx)
}

fn navigate_to_result(
    app: &mut App,
    destination: Location,
    select_name: String,
    tx: &mut Tx,
) -> bool {
    if app.browser.active_location().as_ref() == Some(&destination) {
        if select_name.is_empty() {
            app.archive.pending_destination = None;
            app.archive.pending_select.clear();
            return false;
        }
        app.archive.pending_destination = Some(destination.clone());
        return reveal_compressed(app, destination, select_name, tx);
    }
    if !select_name.is_empty() {
        app.archive.pending_destination = Some(destination.clone());
        app.archive.pending_select = vec![select_name];
    }
    app.browser.navigate(destination);
    shell::settle(app, tx);
    true
}

fn destination_depth(app: &App, destination: &Location) -> Option<usize> {
    if shows_columns(app) {
        app.browser.open_depth(destination)
    } else {
        app.browser
            .active_depth()
            .filter(|depth| app.browser.location_at(*depth).as_ref() == Some(destination))
    }
}

fn reveal_compressed(
    app: &mut App,
    destination: Location,
    select_name: String,
    tx: &mut Tx,
) -> bool {
    app.archive.pending_select = vec![select_name];
    let Some(depth) = destination_depth(app, &destination) else {
        return reload_destination(app, destination, tx);
    };
    if reveal_pending_at(app, depth, tx) {
        return true;
    }
    if !app.browser.column_has_monitor(depth) {
        app.browser.retry_column(depth);
        shell::settle(app, tx);
        return true;
    }
    false
}

fn reload_destination(app: &mut App, destination: Location, tx: &mut Tx) -> bool {
    if shows_columns(app) {
        match app.browser.open_depth(&destination) {
            Some(depth) => {
                app.browser.set_active_column(depth);
                app.browser.retry_column(depth);
            }
            None => app.browser.navigate(destination),
        }
    } else if app.browser.active_location().as_ref() == Some(&destination) {
        app.browser.reload_active();
    } else {
        app.browser.navigate(destination);
    }
    shell::settle(app, tx);
    true
}

fn show_archive_result(
    app: &mut App,
    destination: Option<Location>,
    names: Vec<String>,
    tx: &mut Tx,
) -> bool {
    let active = app.browser.active_location();
    let Some(folder) = destination.or_else(|| active.clone()) else {
        return false;
    };
    let Some(depth) = app.browser.open_depth(&folder) else {
        return false;
    };
    if active.as_ref() == Some(&folder) {
        app.archive.pending_destination = Some(folder);
        app.archive.pending_select = names;
    }
    if app.browser.column_has_monitor(depth) {
        reveal_pending_at(app, depth, tx)
    } else {
        app.browser.retry_column(depth);
        shell::settle(app, tx);
        true
    }
}

/// A batch's results arrive one listing change at a time; selecting waits for the last.
fn pending_listed_at(app: &App, depth: usize) -> bool {
    let pending = &app.archive.pending_select;
    let all_listed = |entries: &[FileEntry]| {
        pending
            .iter()
            .all(|name| entries.iter().any(|entry| entry.display_name == *name))
    };
    app.browser
        .with_column_entries(depth, all_listed)
        .unwrap_or(false)
}

fn reveal_pending_at(app: &mut App, depth: usize, tx: &mut Tx) -> bool {
    let Some(destination) = app.archive.pending_destination.as_ref() else {
        return false;
    };
    if app.browser.location_at(depth).as_ref() != Some(destination) {
        return false;
    }
    if app.archive.pending_select.is_empty() || !pending_listed_at(app, depth) {
        return false;
    }
    if !app
        .browser
        .select_entries_by_name_at(depth, &app.archive.pending_select)
    {
        return false;
    }
    if shows_columns(app) {
        app.browser.set_active_column(depth);
    }
    app.archive.pending_destination = None;
    app.archive.pending_select.clear();
    shell::settle(app, tx);
    true
}
