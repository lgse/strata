// SPDX-License-Identifier: MIT

//! Active chooser request state, selection resolution, filename auto-fill, and acceptance
//! (`src/ui/chooser/state.rs`).

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use rune::{TextEdit, Tx};
use strata_components::{
    contract::EntryKind as NameKind,
    rename::{Initial, initial_selection},
};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_fs::{
    adapters::query_file_entry,
    services::{remote_file_url, validate_basename},
};

use super::{
    ChooserPrompt, NAME_KEY,
    download::{self, DownloadProgress},
    options::{ChoiceControl, build_choices, normalize_portal_filters},
    source::{ChooserFileSource, CompiledFilter},
};
use crate::{
    App, filter,
    portal::{
        ChooserKind, ChooserRequest, Completion, DestinationCheck, PortalError, PortalFileFilter,
        SelectedResult, check_destinations, local_uri, open_selection, safe_filename,
        writable_from_read_only,
    },
    shell,
};

/// A destination check a floating chooser's host runs on the interface thread.
pub(crate) type PendingValidation =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf, String>>>>;

/// What closes a chooser's window once its request finishes, in place of `Tx::close_window`.
pub(crate) type CloseWindow = Box<dyn FnOnce(&mut Tx)>;

pub(crate) type ValidateDestination = Rc<dyn Fn(&Path) -> PendingValidation>;

pub(crate) struct DestinationHost {
    pub(crate) allow_create: bool,
    pub(crate) confined: bool,
    pub(crate) validate: ValidateDestination,
}

pub(crate) struct ActiveChooser {
    pub(crate) request: ChooserRequest,
    pub(crate) source: Rc<ChooserFileSource>,
    pub(crate) destination_host: Option<DestinationHost>,
    pub(crate) has_name: bool,
    pub(crate) name: TextEdit,
    pub(crate) name_placeholder: String,
    pub(crate) name_error: Option<String>,
    pub(crate) filename_selection: Vec<Location>,
    pub(crate) filename_edited: bool,
    pub(crate) filename_source: Option<OsString>,
    pub(crate) filters: Vec<CompiledFilter>,
    pub(crate) filter_selected: Option<usize>,
    pub(crate) filter_open: bool,
    pub(crate) choices: Vec<ChoiceControl>,
    pub(crate) read_only: Option<bool>,
    pub(crate) error: Option<String>,
    pub(crate) destination_check: bool,
    pub(crate) accept_generation: u64,
    pub(crate) accept_sensitive: bool,
    pub(crate) completion: Option<Completion>,
    pub(crate) close_window: Option<CloseWindow>,
    pub(crate) download_cancel: Option<Arc<AtomicBool>>,
    pub(crate) downloaded_file: Option<(String, PathBuf)>,
    pub(crate) download_progress: Option<DownloadProgress>,
    pub(crate) focus_initial: bool,
}

impl ActiveChooser {
    pub(crate) fn new(
        request: ChooserRequest,
        source: Rc<ChooserFileSource>,
        destination_host: Option<DestinationHost>,
        completion: Completion,
    ) -> Self {
        let (filters, filter_selected) =
            normalize_portal_filters(&request.filters, request.current_filter.as_ref());
        source.set_filter(filter_selected.and_then(|idx| filters.get(idx)).cloned());
        source.directory_only.set(matches!(
            &request.kind,
            ChooserKind::Open {
                directory: true,
                ..
            }
        ));
        let (has_name, name, name_placeholder, filename_source) = build_name_state(&request.kind);
        let choices = build_choices(&request.choices);
        let read_only = matches!(
            &request.kind,
            ChooserKind::Open {
                directory: false,
                ..
            }
        )
        .then_some(false);
        Self {
            request,
            source,
            destination_host,
            has_name,
            name,
            name_placeholder,
            name_error: None,
            filename_selection: Vec::new(),
            filename_edited: false,
            filename_source,
            filters,
            filter_selected,
            filter_open: false,
            choices,
            read_only,
            error: None,
            destination_check: false,
            accept_generation: 0,
            accept_sensitive: true,
            completion: Some(completion),
            close_window: None,
            download_cancel: None,
            downloaded_file: None,
            download_progress: None,
            focus_initial: true,
        }
    }

    pub(crate) fn allows_multiple(&self) -> bool {
        matches!(&self.request.kind, ChooserKind::Open { multiple: true, .. })
    }

    pub(crate) fn allows_create(&self) -> bool {
        self.destination_host
            .as_ref()
            .is_none_or(|host| host.allow_create)
    }

    pub(crate) fn is_confined(&self) -> bool {
        self.source.root_limit.is_some()
            || self
                .destination_host
                .as_ref()
                .is_some_and(|host| host.confined)
    }

    pub(crate) fn show_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    pub(crate) fn clear_error(&mut self) {
        self.error = None;
    }

    pub(crate) fn download_in_progress(&self) -> bool {
        self.download_cancel.is_some()
    }

    pub(crate) fn download_is_current(&self, flag: &Arc<AtomicBool>) -> bool {
        self.download_cancel
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, flag))
    }

    pub(crate) fn cancel_download(&mut self) -> bool {
        let Some(cancelled) = self.download_cancel.take() else {
            return false;
        };
        cancelled.store(true, Ordering::SeqCst);
        self.download_progress = None;
        self.accept_sensitive = true;
        true
    }

    pub(crate) fn selected_compiled_filter(&self) -> Option<&CompiledFilter> {
        self.filter_selected.and_then(|idx| self.filters.get(idx))
    }

    pub(crate) fn selected_filter(&self) -> Option<PortalFileFilter> {
        self.selected_compiled_filter()
            .map(|compiled| compiled.portal.clone())
    }

    pub(crate) fn selected_choices(&self) -> Vec<(String, String)> {
        self.choices.iter().map(ChoiceControl::value).collect()
    }

    pub(crate) fn writable(&self) -> Option<bool> {
        self.read_only.map(writable_from_read_only)
    }

    pub(crate) fn name_text(&self) -> Option<String> {
        self.has_name.then(|| self.name.text.clone())
    }

    pub(crate) fn dropdown_open(&self) -> bool {
        self.filter_open || self.choices.iter().any(ChoiceControl::is_open)
    }

    pub(crate) fn dismiss_dropdown(&mut self) -> bool {
        if self.filter_open {
            self.filter_open = false;
            return true;
        }
        self.choices.iter_mut().any(ChoiceControl::close)
    }

    pub(crate) fn fill_filename(&mut self, name: &OsStr) {
        let text = name.to_string_lossy().into_owned();
        self.name.replace_text(text);
        self.filename_source = Some(name.to_owned());
    }

    pub(crate) fn clear_filename(&mut self) {
        self.name.replace_text(String::new());
        self.filename_source = None;
        self.filename_edited = false;
    }

    pub(crate) fn save_destination_name(&self, text: &str) -> OsString {
        match self.filename_source.as_deref() {
            Some(source) if source.to_string_lossy() == text => source.to_owned(),
            _ => OsString::from(text),
        }
    }

    pub(crate) fn select_name_stem(&mut self) {
        let (start, end) = initial_selection(&self.name.text, NameKind::File, Initial::Stem);
        self.name.set_selection(start, end);
    }

    pub(crate) fn finish(
        &mut self,
        result: Result<SelectedResult, PortalError>,
        tx: &mut Tx,
    ) -> bool {
        self.cancel_download();
        let Some(completion) = self.completion.take() else {
            return false;
        };
        if let Some(close) = self.close_window.take() {
            close(tx);
        } else {
            tx.close_window();
        }
        completion(result);
        true
    }
}

fn build_name_state(kind: &ChooserKind) -> (bool, TextEdit, String, Option<OsString>) {
    match kind {
        ChooserKind::SaveFile { current_name } => {
            let mut edit = TextEdit::new(String::new());
            if let Some(initial) = current_name.as_deref() {
                let text = initial.to_string_lossy().into_owned();
                let (start, end) = initial_selection(&text, NameKind::File, Initial::Stem);
                edit.replace_text(text);
                edit.set_selection(start, end);
            }
            (true, edit, tr("Enter a filename"), current_name.clone())
        }
        ChooserKind::Open {
            directory: false, ..
        } => (
            true,
            TextEdit::new(String::new()),
            tr("Enter a filename or https:// URL"),
            None,
        ),
        ChooserKind::Open {
            directory: true, ..
        }
        | ChooserKind::SaveFiles { .. } => {
            (false, TextEdit::new(String::new()), String::new(), None)
        }
    }
}

pub(crate) fn chosen_entries(app: &App, load_cursor: bool) -> Vec<FileEntry> {
    let entries = match filter::targets::selected(app) {
        Some(results) => results,
        None if app.browser.selection_is_load_cursor() => {
            return if load_cursor {
                app.browser.selected_entries()
            } else {
                Vec::new()
            };
        }
        None => app.browser.selected_entries(),
    };
    if entries.is_empty() && shell::preferences(app).tenxer_mode {
        return focused_target(app).into_iter().collect();
    }
    entries
}

fn focused_target(app: &App) -> Option<FileEntry> {
    if let Some((_, entry)) = filter::targets::under_cursor(app) {
        return Some(entry);
    }
    app.browser.focused_entry()
}

pub(crate) fn has_fill(app: &App) -> bool {
    match filter::targets::selected(app) {
        Some(results) => !results.is_empty(),
        None => {
            !app.browser.selection_is_load_cursor() && !app.browser.selected_entries().is_empty()
        }
    }
}

fn name_selection_entries(app: &App) -> Vec<FileEntry> {
    if filter::targets::selected(app).is_some() {
        return chosen_entries(app, true);
    }
    if app.browser.selection_is_load_cursor() && app.browser.selected_entries().len() <= 1 {
        return Vec::new();
    }
    chosen_entries(app, true)
}

pub(crate) fn update_selected_filename(app: &mut App) {
    if shell::preferences(app).tenxer_mode {
        return;
    }
    let has_name = app.chooser.active.as_ref().is_some_and(|a| a.has_name);
    if !has_name {
        return;
    }
    let entries = name_selection_entries(app);
    let selected: Vec<Location> = entries.iter().map(|entry| entry.location.clone()).collect();
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    if std::mem::replace(&mut active.filename_selection, selected.clone()) == selected {
        return;
    }
    if let Some(name) = single_safe_file_name(&entries) {
        active.fill_filename(name);
        active.filename_edited = false;
        active.name_error = None;
    } else if matches!(
        active.request.kind,
        ChooserKind::Open {
            directory: false,
            ..
        }
    ) {
        active.clear_filename();
    }
}

pub(crate) fn name_changed(app: &mut App) {
    let entries = name_selection_entries(app);
    if let Some(active) = app.chooser.active.as_mut() {
        active.filename_selection = entries.into_iter().map(|entry| entry.location).collect();
        active.filename_edited = true;
        active.name_error = None;
    }
}

fn single_safe_file_name(entries: &[FileEntry]) -> Option<&OsStr> {
    let [entry] = entries else {
        return None;
    };
    if entry.is_directory() {
        return None;
    }
    entry
        .location
        .native_path()
        .and_then(Path::file_name)
        .filter(|name| safe_filename(name))
}

fn selected_folder(app: &App) -> Option<PathBuf> {
    if shell::preferences(app).tenxer_mode {
        return None;
    }
    let entries = chosen_entries(app, false);
    if entries.len() == 1 && entries[0].is_directory() {
        entries[0].location.native_path().map(Path::to_path_buf)
    } else {
        None
    }
}

fn recent_entry_parent(app: &App) -> Option<PathBuf> {
    let in_recent = app
        .browser
        .active_location()
        .is_some_and(|location| location.is_recent_root());
    if !in_recent {
        return None;
    }
    let entries = chosen_entries(app, false);
    let [entry] = entries.as_slice() else {
        return None;
    };
    entry
        .location
        .native_path()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

pub(crate) fn active_folder(app: &App) -> Result<PathBuf, &'static str> {
    if let Some(folder) = selected_folder(app) {
        return Ok(folder);
    }
    if let Some(parent) = recent_entry_parent(app) {
        return Ok(parent);
    }
    app.browser
        .active_location()
        .and_then(|location| location.native_path().map(Path::to_path_buf))
        .ok_or("Choose an accessible local folder")
}

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
    let mut uris = Vec::with_capacity(paths.len());
    for path in paths {
        match local_uri(&path) {
            Ok(uri) => uris.push(uri),
            Err(error) => {
                active.finish(Err(error), tx);
                return;
            }
        }
    }
    let result = SelectedResult {
        uris,
        choices: active.selected_choices(),
        current_filter: active.selected_filter(),
        writable,
    };
    active.finish(Ok(result), tx);
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
    let modal = shell::modal_open(app);
    let blocked = app.chooser.active.as_ref().is_none_or(|active| {
        active.completion.is_none()
            || active.destination_check
            || active.download_in_progress()
            || modal
    });
    if blocked {
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
        ChooserKind::SaveFiles { names } => match active_folder(app) {
            Ok(folder) => accept_destinations(app, folder, names),
            Err(message) => {
                if let Some(active) = app.chooser.active.as_mut() {
                    active.show_error(tr(message));
                }
            }
        },
    }
}

fn accept_open(app: &mut App, directory: bool, multiple: bool, tx: &mut Tx) {
    update_selected_filename(app);
    let edited = app.chooser.active.as_ref().and_then(|active| {
        (!directory && active.filename_edited && active.has_name && !active.name.text.is_empty())
            .then(|| active.name.text.clone())
    });
    if let Some(name) = edited {
        accept_open_name(app, &name, tx);
        return;
    }
    let Some(current) = app.browser.active_location() else {
        if let Some(active) = app.chooser.active.as_mut() {
            active.show_error(tr("Choose an accessible local folder"));
        }
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
            if let Some(active) = app.chooser.active.as_mut() {
                active.show_error(tr(message));
            }
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
            let Some(active) = app.chooser.active.as_mut() else {
                return false;
            };
            active.destination_check = false;
            if active.completion.is_none() || active.accept_generation != generation {
                return false;
            }
            active.accept_sensitive = true;
            match result {
                Ok(path) => complete_paths(app, vec![path], None, tx),
                Err(message) => active.show_error(message),
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
        if let Some(active) = app.chooser.active.as_mut() {
            active.show_error(tr(message));
        }
        return;
    }
    let folder = app
        .browser
        .active_location()
        .and_then(|loc| loc.native_path().map(Path::to_path_buf))
        .or_else(|| active_folder(app).ok());
    let Some(folder) = folder else {
        if let Some(active) = app.chooser.active.as_mut() {
            active.show_error(tr("Choose an accessible local folder"));
        }
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
    let valid = app.chooser.active.as_mut().is_some_and(|active| {
        active.destination_check = false;
        if active.completion.is_none() || active.accept_generation != generation {
            return false;
        }
        active.accept_sensitive = true;
        true
    });
    if !valid {
        return false;
    }
    let Ok(entry) = result else {
        if let Some(active) = app.chooser.active.as_mut() {
            active.show_error(tr("Choose an existing, accessible file"));
        }
        return true;
    };
    if entry.is_directory() {
        app.browser.navigate(entry.location);
        if let Some(active) = app.chooser.active.as_mut() {
            active.clear_filename();
        }
        return true;
    }
    if !app.browser.allows_entry(&entry) {
        if let Some(active) = app.chooser.active.as_mut() {
            active.show_error(tr("The file does not match the selected filter"));
        }
        return true;
    }
    match entry.location.native_path() {
        Some(path) => download::finish_remote(app, path.to_path_buf(), tx),
        None => {
            if let Some(active) = app.chooser.active.as_mut() {
                active.show_error(tr("Choose an existing, accessible file"));
            }
        }
    }
    true
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
            if let Some(active) = app.chooser.active.as_mut() {
                active.show_error(tr(message));
            }
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
    let valid = app.chooser.active.as_mut().is_some_and(|active| {
        active.destination_check = false;
        if active.accept_generation != generation {
            return false;
        }
        active.accept_sensitive = true;
        active.completion.is_some()
    });
    if !valid {
        return false;
    }
    let destinations = match result {
        Ok(destinations) => destinations,
        Err(message) => {
            if let Some(active) = app.chooser.active.as_mut() {
                active.show_error(message);
            }
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
    let allowed = app
        .chooser
        .active
        .as_ref()
        .is_some_and(|active| active.completion.is_some())
        && app.browser.allows_entry(entry);
    if !allowed {
        return;
    }
    let kind = app
        .chooser
        .active
        .as_ref()
        .expect("checked active")
        .request
        .kind
        .clone();
    if matches!(
        kind,
        ChooserKind::Open {
            directory: true,
            ..
        }
    ) {
        if let Some(active) = app.chooser.active.as_mut() {
            active.show_error(tr("Choose folders only"));
        }
        return;
    }
    if matches!(
        kind,
        ChooserKind::Open {
            directory: false,
            multiple: true,
        }
    ) && has_fill(app)
    {
        accept(app, tx);
        return;
    }
    activate_file(app, &entry.location, tx);
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
            None => {
                if let Some(active) = app.chooser.active.as_mut() {
                    active.show_error(tr("Choose a local file"));
                }
            }
        },
        ChooserKind::SaveFile { .. } => {
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
        _ => {}
    }
}
