// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;
mod view;

use rune::{TextEdit, Tx};
use strata_core::{
    i18n::tr,
    model::{ArchiveFormat, FileEntry, Location},
};
use strata_fs::{app::BrowserEvent, services::PasswordFailure};

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
    let entry = match call.on {
        On::Row((depth, id)) => menus::context_entries(app, depth, id).map(|(entry, _)| entry),
        On::Focus => focused_archive(app),
        On::Column(_) | On::Place(_) => None,
    };
    if let Some(entry) = entry.filter(|entry| ArchiveFormat::for_entry(entry).is_some()) {
        extract_entry(app, entry, tx);
    }
    Outcome::Ran
}

fn focused_archive(app: &mut App) -> Option<FileEntry> {
    shell::publish(app);
    let entries = app.browser.selected_entries();
    match entries.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
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

pub(crate) fn abandon_follow_ups(app: &mut App) {
    let archive = &mut app.archive;
    archive.pending_destination = None;
    archive.pending_select.clear();
    archive.extract_destination = None;
    archive.pending_extract_retry = None;
    archive.pending_navigate = None;
}

pub(crate) fn on_operation_failed(
    app: &mut App,
    password_failure: Option<PasswordFailure>,
    tx: &mut Tx,
) -> bool {
    let archive = &mut app.archive;
    archive.pending_destination = None;
    archive.pending_select.clear();
    archive.extract_destination = None;
    let retry = archive.pending_extract_retry.take();
    let navigate_after_extract = archive.pending_navigate.take();
    let Some((entry, destination)) = retry else {
        return false;
    };
    let Some(invalid_password) = extract_password_retry(password_failure) else {
        return false;
    };
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

fn cancel(app: &mut App) {
    app.archive.open = false;
    if let Some(form) = &mut app.archive.form {
        form.wipe_secret();
    }
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

pub(crate) fn on_completed(app: &mut App, select_name: String, tx: &mut Tx) -> bool {
    app.archive.pending_extract_retry = None;
    let extract_destination = app.archive.extract_destination.take();
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
            show_archive_result(app, extract_destination, select_name, tx)
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
    name: String,
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
        app.archive.pending_select = vec![name];
    }
    if app.browser.column_has_monitor(depth) {
        reveal_pending_at(app, depth, tx)
    } else {
        app.browser.retry_column(depth);
        shell::settle(app, tx);
        true
    }
}

fn reveal_pending_at(app: &mut App, depth: usize, tx: &mut Tx) -> bool {
    let Some(destination) = app.archive.pending_destination.as_ref() else {
        return false;
    };
    if app.browser.location_at(depth).as_ref() != Some(destination) {
        return false;
    }
    if app.archive.pending_select.is_empty() {
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
