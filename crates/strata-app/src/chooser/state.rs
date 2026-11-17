// SPDX-License-Identifier: MIT

//! Active chooser request state, selection resolution and filename auto-fill
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

use super::{
    download::DownloadProgress,
    options::{ChoiceControl, build_choices, normalize_portal_filters},
    source::{ChooserFileSource, CompiledFilter},
};
use crate::{
    App, filter,
    portal::{
        ChooserKind, ChooserRequest, Completion, PortalError, PortalFileFilter, SelectedResult,
        safe_filename, writable_from_read_only,
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

    /// Whether Open or Save may run now: the request waits for an answer, and no destination
    /// check or download is under way.
    pub(crate) fn can_accept(&self) -> bool {
        self.completion.is_some() && !self.destination_check && !self.download_in_progress()
    }

    pub(crate) fn download_in_progress(&self) -> bool {
        self.download_cancel.is_some()
    }

    pub(crate) fn download_is_current(&self, flag: &Arc<AtomicBool>) -> bool {
        self.download_cancel
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, flag))
    }

    /// The download from `url` finished at `path`, which a later accept of the same URL reuses.
    pub(crate) fn keep_download(&mut self, url: &str, path: &Path) {
        self.download_progress = None;
        self.download_cancel = None;
        self.downloaded_file = Some((url.to_owned(), path.to_owned()));
    }

    /// The kept download moved from `from` to `to`, as a rename for the filter does.
    pub(crate) fn rename_download(&mut self, from: &Path, to: &Path) {
        if let Some((_, kept)) = self.downloaded_file.as_mut()
            && kept.as_path() == from
        {
            *kept = to.to_owned();
        }
    }

    /// The download stopped with `message`, which the footer shows.
    pub(crate) fn fail_download(&mut self, message: String) {
        self.download_progress = None;
        self.accept_sensitive = true;
        self.download_cancel = None;
        self.show_error(message);
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

    /// The choice at `index`, as a control's handler reaches it.
    pub(crate) fn choice_mut(&mut self, index: usize) -> Option<&mut ChoiceControl> {
        self.choices.get_mut(index)
    }

    /// Opens the select at `index` and closes any other list, or closes it when it is open.
    pub(crate) fn toggle_choice_list(&mut self, index: usize) {
        let was_open = self.choices.get(index).is_some_and(ChoiceControl::is_open);
        self.dismiss_dropdown();
        if let Some(choice) = self.choices.get_mut(index) {
            choice.set_open(!was_open);
        }
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
