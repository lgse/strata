// SPDX-License-Identifier: MIT

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use rune::Tx;
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_fs::{
    adapters::query_file_entry,
    services::{remote_file_url, validate_basename},
};

use super::{
    ChooserPrompt, NAME_KEY, download,
    state::{
        ActiveChooser, PendingValidation, active_folder, chosen_entries, has_fill,
        update_selected_filename,
    },
};
use crate::{
    App,
    portal::{
        ChooserKind, DestinationCheck, PortalError, SelectedResult, check_destinations, local_uri,
        open_selection, safe_filename,
    },
    shell,
};

pub(crate) fn complete_paths_writable(app: &mut App, paths: Vec<PathBuf>, tx: &mut Tx) {
    let writable = app
        .chooser
        .active
        .as_ref()
        .and_then(ActiveChooser::writable);
    complete_paths(app, paths, writable, tx);
}

pub(crate) fn complete_paths(
    app: &mut App,
    paths: Vec<PathBuf>,
    writable: Option<bool>,
    tx: &mut Tx,
) {
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    let result = paths
        .iter()
        .map(|path| local_uri(path))
        .collect::<Result<Vec<_>, _>>()
        .map(|uris| SelectedResult {
            uris,
            choices: active.selected_choices(),
            current_filter: active.selected_filter(),
            writable,
        });
    active.finish(result, tx);
}

pub(crate) fn cancel(app: &mut App, tx: &mut Tx) -> bool {
    let Some(active) = app.chooser.active.as_mut() else {
        return false;
    };
    active.finish(
        Err(PortalError::Cancelled("file chooser dismissed".into())),
        tx,
    )
}

pub(crate) fn accept(app: &mut App, tx: &mut Tx) {
    let ready = app
        .chooser
        .active
        .as_ref()
        .is_some_and(ActiveChooser::can_accept);
    if !ready || shell::modal_open(app) {
        return;
    }
    let kind = {
        let active = app.chooser.active.as_mut().expect("checked active");
        active.clear_error();
        active.request.kind.clone()
    };
    match kind {
        ChooserKind::Open {
            directory,
            multiple,
        } => accept_open(app, directory, multiple, tx),
        ChooserKind::SaveFile { .. } => accept_save_file(app, tx),
        ChooserKind::SaveFiles { names } => accept_save_files(app, names),
    }
}

fn accept_save_files(app: &mut App, names: Vec<OsString>) {
    match active_folder(app) {
        Ok(folder) => accept_destinations(app, folder, names),
        Err(message) => app.chooser.show_error(tr(message)),
    }
}

/// The name typed into an Open chooser's field, which it opens instead of the selection.
fn edited_name(app: &App, directory: bool) -> Option<String> {
    let active = app.chooser.active.as_ref()?;
    let typed = active.filename_edited && active.has_name && !active.name.text.is_empty();
    (typed && !directory).then(|| active.name.text.clone())
}

fn accept_open(app: &mut App, directory: bool, multiple: bool, tx: &mut Tx) {
    update_selected_filename(app);
    if let Some(name) = edited_name(app, directory) {
        accept_open_name(app, &name, tx);
        return;
    }
    let Some(current) = app.browser.active_location() else {
        app.chooser
            .show_error(tr("Choose an accessible local folder"));
        return;
    };
    let entries: Vec<FileEntry> = chosen_entries(app, !directory)
        .into_iter()
        .filter(|entry| entry.is_directory() == directory && app.browser.allows_entry(entry))
        .collect();
    let validate = app
        .chooser
        .active
        .as_ref()
        .and_then(|active| active.destination_host.as_ref())
        .map(|host| host.validate.clone());
    match (
        open_selection(&entries, &current, directory, multiple),
        validate,
    ) {
        (Ok(paths), Some(validate)) => accept_destination_host(app, validate(&paths[0])),
        (Ok(paths), None) => complete_paths_writable(app, paths, tx),
        (Err(message), _) => {
            app.chooser.show_error(tr(message));
        }
    }
}

fn accept_destination_host(app: &mut App, validation: PendingValidation) {
    let later = app.chooser.later.clone();
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    active.destination_check = true;
    active.accept_sensitive = false;
    let generation = active.accept_generation;
    glib::MainContext::default().spawn_local(async move {
        let result = validation.await;
        later.then(move |app: &mut App, tx: &mut Tx| {
            if !app.chooser.end_check(generation) {
                return false;
            }
            match result {
                Ok(path) => complete_paths(app, vec![path], None, tx),
                Err(message) => app.chooser.show_error(message),
            }
            true
        });
    });
}

fn accept_open_name(app: &mut App, name: &str, tx: &mut Tx) {
    if let Some(url) = remote_file_url(name) {
        download::open_remote(app, &url, tx);
        return;
    }
    if let Err(message) = validate_basename(name) {
        app.chooser.show_error(tr(message));
        return;
    }
    let folder = app
        .browser
        .active_location()
        .and_then(|loc| loc.native_path().map(Path::to_path_buf))
        .or_else(|| active_folder(app).ok());
    let Some(folder) = folder else {
        app.chooser
            .show_error(tr("Choose an accessible local folder"));
        return;
    };
    let location = Location::local(folder.join(name));
    let later = app.chooser.later.clone();
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    active.destination_check = true;
    active.accept_sensitive = false;
    let generation = active.accept_generation;
    glib::MainContext::default().spawn_local(async move {
        let result = query_file_entry(location).await;
        later.then(move |app: &mut App, tx: &mut Tx| {
            finish_open_named_entry(app, generation, result, tx)
        });
    });
}

fn finish_open_named_entry(
    app: &mut App,
    generation: u64,
    result: Result<FileEntry, glib::Error>,
    tx: &mut Tx,
) -> bool {
    if !app.chooser.end_check(generation) {
        return false;
    }
    match result {
        Ok(entry) => open_named_entry(app, entry, tx),
        Err(_) => app
            .chooser
            .show_error(tr("Choose an existing, accessible file")),
    }
    true
}

/// The entry a typed name names: a folder opens, a file the filter accepts is chosen.
fn open_named_entry(app: &mut App, entry: FileEntry, tx: &mut Tx) {
    if entry.is_directory() {
        app.browser.navigate(entry.location);
        if let Some(active) = app.chooser.active.as_mut() {
            active.clear_filename();
        }
        return;
    }
    if !app.browser.allows_entry(&entry) {
        app.chooser
            .show_error(tr("The file does not match the selected filter"));
        return;
    }
    match entry.location.native_path() {
        Some(path) => download::finish_remote(app, path.to_path_buf(), tx),
        None => app
            .chooser
            .show_error(tr("Choose an existing, accessible file")),
    }
}

fn accept_save_file(app: &mut App, tx: &mut Tx) {
    let Some(name) = app
        .chooser
        .active
        .as_ref()
        .and_then(ActiveChooser::name_text)
    else {
        return;
    };
    if let Err(message) = validate_basename(&name) {
        let translated = tr(message);
        if let Some(active) = app.chooser.active.as_mut() {
            active.name_error = Some(translated.clone());
            active.show_error(translated);
            active.name.focused = true;
        }
        tx.focus(NAME_KEY, true);
        return;
    }
    if let Some(active) = app.chooser.active.as_mut() {
        active.name_error = None;
    }
    let folder = match active_folder(app) {
        Ok(folder) => folder,
        Err(message) => {
            app.chooser.show_error(tr(message));
            return;
        }
    };
    let Some(destination_name) = app
        .chooser
        .active
        .as_ref()
        .map(|active| active.save_destination_name(&name))
    else {
        return;
    };
    accept_destinations(app, folder, vec![destination_name]);
}

pub(crate) fn accept_destinations(app: &mut App, folder: PathBuf, names: Vec<OsString>) {
    let later = app.chooser.later.clone();
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    if std::mem::replace(&mut active.destination_check, true) {
        return;
    }
    active.accept_sensitive = false;
    let generation = active.accept_generation;
    glib::MainContext::default().spawn_local(async move {
        let result = check_destinations(&folder, &names).await;
        later.then(move |app: &mut App, tx: &mut Tx| {
            finish_destination_check(app, generation, result, tx)
        });
    });
}

fn finish_destination_check(
    app: &mut App,
    generation: u64,
    result: Result<DestinationCheck, String>,
    tx: &mut Tx,
) -> bool {
    if !app.chooser.end_check(generation) {
        return false;
    }
    let destinations = match result {
        Ok(destinations) => destinations,
        Err(message) => {
            app.chooser.show_error(message);
            return true;
        }
    };
    if destinations.existing.is_empty() {
        complete_paths(app, destinations.paths, None, tx);
    } else {
        app.chooser_dialog
            .open(ChooserPrompt::Overwrite(destinations));
    }
    true
}

pub(crate) fn confirm_file(app: &mut App, entry: &FileEntry, tx: &mut Tx) {
    let Some(kind) = pending_kind(app) else {
        return;
    };
    if !app.browser.allows_entry(entry) {
        return;
    }
    match kind {
        ChooserKind::Open {
            directory: true, ..
        } => app.chooser.show_error(tr("Choose folders only")),
        ChooserKind::Open { multiple: true, .. } if has_fill(app) => accept(app, tx),
        _ => activate_file(app, &entry.location, tx),
    }
}

/// The kind of the chooser's request, while it waits for an answer.
fn pending_kind(app: &App) -> Option<ChooserKind> {
    let active = app.chooser.active.as_ref()?;
    active
        .completion
        .is_some()
        .then(|| active.request.kind.clone())
}

pub(crate) fn activate_file(app: &mut App, location: &Location, tx: &mut Tx) {
    let kind = match app.chooser.active.as_ref() {
        Some(active) if !active.download_in_progress() => active.request.kind.clone(),
        _ => return,
    };
    match kind {
        ChooserKind::Open {
            directory: false, ..
        } => match location.native_path() {
            Some(path) => complete_paths_writable(app, vec![path.to_path_buf()], tx),
            None => app.chooser.show_error(tr("Choose a local file")),
        },
        ChooserKind::SaveFile { .. } => save_onto(app, location),
        _ => {}
    }
}

/// A Save chooser's file activated: its name fills the field, and the save goes to it.
fn save_onto(app: &mut App, location: &Location) {
    let Some((folder, name)) = location
        .native_path()
        .and_then(|path| Some((path.parent()?.to_owned(), path.file_name()?.to_owned())))
        .filter(|(_, name)| safe_filename(name))
    else {
        return;
    };
    if let Some(active) = app.chooser.active.as_mut()
        && active.has_name
    {
        active.fill_filename(&name);
    }
    accept_destinations(app, folder, vec![name]);
}
