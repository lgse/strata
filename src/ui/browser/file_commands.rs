// SPDX-License-Identifier: MIT

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use super::{
    BrowserView, PinStatus,
    clipboard::{self, copy_locations, copy_names},
    desktop::{can_open_terminal, launch_terminal},
    destination::resolve_destination_path,
    paths::{can_remove_location, is_trash_item, is_trash_location},
};
use crate::{
    adapters::gio_file_for_location,
    model::{FileEntry, Location, SortDirection, SortKey},
    services::{ActionHandle, ArchiveFormat, InvocationSource},
};

pub(crate) use super::transfer::ConflictFocus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Yank {
    Nothing,
    Done,
    Refused,
}

pub(crate) struct NumberedActions {
    pub(crate) targets: Vec<Location>,
    pub(crate) actions: Vec<Rc<ActionHandle>>,
    paths: Vec<PathBuf>,
    parent: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KeyboardRefocus {
    Sort(usize),
    Rename(crate::services::OperationRequestId),
}

/// What **g +** / **g -** did to the folder they chose, named by its display name.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PinChange {
    Pinned(Location, String),
    Unpinned(String),
    AlreadyPinned(String),
    NotPinned(String),
    /// Standard places and Trash already have their own sidebar rows.
    Refused(String),
    Nothing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetCommand {
    Nothing,
    Refused,
    Started,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CreateRefusal {
    Unsupported,
    Invalid(&'static str),
    Exists(String),
}

impl BrowserView {
    pub fn command_targets(&self) -> Vec<FileEntry> {
        if let Some(results) = self.selected_search_results() {
            if !results.is_empty() {
                return results;
            }
            return self.selected_search_result().into_iter().collect();
        }
        self.state.sync_mode_selection();
        self.focused_listing_depth()
            .map(|depth| self.state.browser.command_entries(depth))
            .unwrap_or_default()
    }

    pub fn focused_target(&self) -> Option<FileEntry> {
        if self.selected_search_results().is_some() {
            return self.selected_search_result();
        }
        self.state.sync_mode_selection();
        self.focused_listing_depth()
            .and_then(|depth| self.state.browser.cursor_entry(depth))
    }

    pub fn yank_targets(&self, cut: bool) -> Yank {
        let entries = self.command_targets();
        if entries.is_empty() {
            return Yank::Nothing;
        }
        if !cut {
            self.state.copy_entries(&entries);
            return Yank::Done;
        }
        if self.state.cut_entries(&entries) {
            Yank::Done
        } else {
            Yank::Refused
        }
    }

    pub fn unyank(&self) {
        clipboard::unyank();
    }

    pub fn copy_target_text(&self, names: bool) -> bool {
        let entries = self.command_targets();
        if entries.is_empty() {
            return false;
        }
        if names {
            copy_names(&entries);
        } else {
            copy_locations(&entries);
        }
        true
    }

    pub fn paste_preferring(&self, focus: ConflictFocus, nothing: Rc<dyn Fn()>) {
        if !clipboard::clipboard_may_paste() {
            nothing();
            return;
        }
        if let Some(location) = self.paste_location() {
            self.state.paste_preferring(location, focus, nothing);
        }
    }

    pub fn confirm_targets_delete(&self, permanent: bool) -> bool {
        let entries = self.command_targets();
        if entries.is_empty() {
            return false;
        }
        let permanent = permanent || self.focused_location_is_trash();
        self.state.request_confirmed_delete(entries, permanent);
        true
    }

    pub fn can_create_entry(&self) -> bool {
        self.new_entry_parent()
            .is_some_and(|(_, parent)| !is_trash_location(&parent) && !parent.is_recent_location())
    }

    pub fn create_typed_entry(&self, text: &str) -> Result<(), CreateRefusal> {
        let (name, directory) = match text.strip_suffix('/') {
            Some(name) => (name, true),
            None => (text, false),
        };
        crate::services::validate_basename(name).map_err(CreateRefusal::Invalid)?;
        let Some((_, parent)) = self
            .new_entry_parent()
            .filter(|(_, parent)| !is_trash_location(parent) && !parent.is_recent_location())
        else {
            return Err(CreateRefusal::Unsupported);
        };
        // The operation still checks collisions atomically; this provides editable feedback.
        if let Some(path) = parent.native_path()
            && std::fs::symlink_metadata(path.join(name)).is_ok()
        {
            return Err(CreateRefusal::Exists(name.to_owned()));
        }
        self.state
            .browser
            .create_exact_entry(parent, name.to_owned(), directory);
        Ok(())
    }
}

pub(crate) fn can_rename(entry: &FileEntry) -> bool {
    !is_trash_location(&entry.location)
}

impl BrowserView {
    /// Renames `entry` to exactly `text`. An unchanged name does nothing, and an
    /// invalid or occupied one is refused before anything is renamed; the
    /// operation's own failures are reported like an inline rename's.
    pub fn rename_typed_entry(&self, entry: FileEntry, text: &str) -> Result<(), CreateRefusal> {
        if !can_rename(&entry) {
            return Err(CreateRefusal::Unsupported);
        }
        crate::services::validate_basename(text).map_err(CreateRefusal::Invalid)?;
        if entry.display_name == text || entry.native_name == std::ffi::OsStr::new(text) {
            return Ok(());
        }
        // The rename itself refuses to replace; this only keeps the prompt open to fix the name.
        if let Some(parent) = entry
            .location
            .native_path()
            .and_then(std::path::Path::parent)
            && std::fs::symlink_metadata(parent.join(text)).is_ok()
        {
            return Err(CreateRefusal::Exists(text.to_owned()));
        }
        if let Some(request) = self.state.browser.rename(entry, text.to_owned()) {
            self.state
                .keyboard_refocus
                .set(Some(KeyboardRefocus::Rename(request)));
        }
        Ok(())
    }
}

impl super::ViewState {
    pub(super) fn finish_keyboard_refocus(
        self: &Rc<Self>,
        finished: KeyboardRefocus,
        completed: bool,
    ) {
        if self.keyboard_refocus.get() != Some(finished) {
            return;
        }
        self.keyboard_refocus.set(None);
        if completed {
            self.refocus_cursor();
        }
    }

    fn refocus_cursor(self: &Rc<Self>) {
        const FRAMES: u8 = 12;
        let state = Rc::downgrade(self);
        let frames = std::cell::Cell::new(0u8);
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // A replaced row briefly keeps focus outside its list; wait it out.
            let on_pane = state
                .overlay
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focus| {
                    focus.is::<gtk::Stack>()
                        || focus.is::<gtk::ListView>()
                        || focus.is::<gtk::GridView>()
                });
            if on_pane
                && (BrowserView {
                    state: state.clone(),
                })
                .item_view_has_focus()
            {
                state.browser.focus_active();
            }
            frames.set(frames.get() + 1);
            if frames.get() < FRAMES {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
}

impl BrowserView {
    pub fn sort_focused_pane(&self, key: SortKey, direction: SortDirection) {
        if let Some(depth) = self.focused_listing_depth() {
            self.state.browser.set_sort(depth, key, direction);
            self.state
                .keyboard_refocus
                .set(Some(KeyboardRefocus::Sort(depth)));
        }
    }

    /// Opens a terminal in the keyboard-focused pane's folder, ignoring the
    /// cursor and selection. Returns false where no terminal can open.
    pub fn open_focused_folder_terminal(&self) -> bool {
        let Some(location) = self
            .focused_listing_depth()
            .and_then(|depth| self.state.browser.location_at(depth))
            .filter(can_open_terminal)
        else {
            return false;
        };
        launch_terminal(&location, &self.state.overlay);
        true
    }

    pub fn toggle_hidden_from_keys(&self) {
        self.state.browser.toggle_hidden();
        self.state.refocus_cursor();
    }

    pub(crate) fn numbered_actions(&self) -> NumberedActions {
        let entries = self.command_targets();
        let targets = entries.iter().map(|entry| entry.location.clone()).collect();
        let parent = self
            .focused_listing_depth()
            .and_then(|depth| self.state.browser.location_at(depth))
            .and_then(|location| location.native_path().map(PathBuf::from));
        let (inputs, paths) = match (
            crate::ui::actions::inputs_for_entries(&entries),
            crate::ui::actions::native_paths(&entries),
        ) {
            (Some(inputs), Some(paths)) if parent.is_some() => (inputs, paths),
            _ => (Vec::new(), Vec::new()),
        };
        let actions = if inputs.is_empty() {
            Vec::new()
        } else {
            crate::ui::actions::shared().catalog().numbered(&inputs)
        };
        NumberedActions {
            targets,
            actions,
            paths,
            parent,
        }
    }

    pub(crate) fn run_numbered_action(&self, numbered: NumberedActions, action: Rc<ActionHandle>) {
        let Some(parent) = numbered.parent else {
            return;
        };
        let browser = Rc::downgrade(&self.state.browser);
        crate::ui::actions::run_action(
            &self.state.overlay,
            action,
            numbered.paths,
            parent,
            InvocationSource::Selection,
            Some(Rc::new(move || {
                if let Some(browser) = browser.upgrade() {
                    browser.focus_active();
                }
            })),
        );
    }

    /// Looks up applications for the fill, or the focused item, off the key
    /// handler, then shows Open With over them. The chooser appears only while
    /// `current` holds and the targets and folder are unchanged; otherwise the
    /// lookup ends silently. `report` receives why nothing can be chosen.
    /// `None` means there was nothing to open.
    pub fn open_targets_with(
        &self,
        current: Rc<dyn Fn() -> bool>,
        report: Rc<dyn Fn(&str)>,
    ) -> Option<glib::JoinHandle<()>> {
        let entries = self.command_targets();
        if entries.is_empty() {
            return None;
        }
        let locations: Vec<Location> = entries.iter().map(|entry| entry.location.clone()).collect();
        let files: Vec<_> = locations.iter().map(gio_file_for_location).collect();
        let folder = self.state.browser.active_location();
        let state = Rc::downgrade(&self.state);
        Some(glib::MainContext::default().spawn_local(async move {
            let unchanged = || {
                current()
                    && state.upgrade().is_some_and(|state| {
                        let view = BrowserView { state };
                        view.state.browser.active_location() == folder
                            && view
                                .command_targets()
                                .iter()
                                .map(|entry| &entry.location)
                                .eq(locations.iter())
                    })
            };
            let Some(resolved) = crate::ui::open_with::resolve(&files, &unchanged).await else {
                return;
            };
            let Some(state) = state.upgrade() else {
                return;
            };
            let applications = match resolved {
                Ok(applications) => applications,
                Err(reason) => return report(reason),
            };
            if let Some(reason) = applications.unavailable_reason() {
                return report(reason);
            }
            let browser = Rc::downgrade(&state.browser);
            crate::ui::open_with::show(
                &state.overlay,
                files,
                applications.content_types,
                applications.recommended,
                applications.other,
                crate::ui::open_with::OpenWithContext::Explicit,
                Rc::new(move || {
                    if let Some(browser) = browser.upgrade() {
                        browser.focus_active();
                    }
                }),
            );
        }))
    }
}

impl BrowserView {
    /// The folder under the cursor, or the focused pane's own folder when the
    /// cursor is on a file or the pane is empty.
    fn keyboard_pin_folder(&self) -> Option<(Location, String)> {
        if let Some(entry) = self.focused_target().filter(FileEntry::is_directory) {
            return Some((entry.location, entry.display_name));
        }
        let location = self
            .focused_listing_depth()
            .and_then(|depth| self.state.browser.location_at(depth))?;
        let name = location.display_name();
        Some((location, name))
    }

    pub fn change_keyboard_pin(&self, pin: bool) -> PinChange {
        let Some((location, name)) = self.keyboard_pin_folder() else {
            return PinChange::Nothing;
        };
        let status = if is_trash_location(&location) {
            PinStatus::Unavailable
        } else {
            self.state
                .pin_status_handler
                .borrow()
                .as_ref()
                .map_or(PinStatus::Unavailable, |handler| handler(&location))
        };
        match (pin, status) {
            (true, PinStatus::Available) => {
                let handler = self.state.pin_handler.borrow().clone();
                let Some(handler) = handler else {
                    return PinChange::Refused(name);
                };
                handler(location.clone(), name.clone());
                PinChange::Pinned(location, name)
            }
            (true, PinStatus::Pinned) => PinChange::AlreadyPinned(name),
            (true, PinStatus::Unavailable) => PinChange::Refused(name),
            (false, PinStatus::Pinned) => {
                let handler = self.state.unpin_handler.borrow().clone();
                let Some(handler) = handler else {
                    return PinChange::NotPinned(name);
                };
                handler(&location);
                PinChange::Unpinned(name)
            }
            (false, _) => PinChange::NotPinned(name),
        }
    }

    /// Resolves a typed destination like the move/copy dialog does: absolute,
    /// `~`-relative, or relative to the open local folder. `Err` is the reason
    /// to show beside the prompt.
    pub fn typed_destination_folder(&self, text: &str) -> Result<PathBuf, &'static str> {
        let text = text.trim();
        if crate::ui::go_completion::looks_like_uri(text) {
            return Err("Only local folders can be chosen");
        }
        if text.starts_with('~') && text != "~" && !text.starts_with("~/") {
            return Err("Only ~ and ~/ are supported");
        }
        let home = glib::home_dir();
        let base = self
            .state
            .browser
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf));
        let relative = !text.starts_with('~') && Path::new(text).is_relative();
        let base = match base {
            Some(base) => base,
            None if relative => return Err("Type a full path here"),
            None => home.clone(),
        };
        let path = resolve_destination_path(text, &base, &home);
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => Ok(path),
            Ok(_) => Err("Not a folder"),
            Err(_) => Err("No such folder"),
        }
    }

    /// The fill, or the focused item, for **M** / **C**, fixed when the prompt opens.
    pub fn transfer_targets(&self, moving: bool) -> Result<Vec<FileEntry>, &'static str> {
        let entries = self.command_targets();
        if entries.is_empty() {
            return Err(if moving {
                "Nothing to move"
            } else {
                "Nothing to copy"
            });
        }
        if moving
            && !entries
                .iter()
                .all(|entry| can_remove_location(&entry.location))
        {
            return Err("Can\u{2019}t move these items");
        }
        Ok(entries)
    }

    /// Moves or copies `entries` into the typed folder and stays in the current
    /// one. Conflicts ask as a paste does, with Keep Both focused when offered.
    pub fn transfer_to_typed(
        &self,
        entries: Vec<FileEntry>,
        text: &str,
        moving: bool,
    ) -> Result<(), &'static str> {
        let destination = self.typed_destination_folder(text)?;
        if entries.iter().any(|entry| {
            entry.is_directory()
                && entry
                    .location
                    .native_path()
                    .is_some_and(|source| destination.starts_with(source))
        }) {
            return Err("Can\u{2019}t put a folder inside itself");
        }
        self.state.start_transfer_in_place(
            Location::local(destination),
            entries.into_iter().map(|entry| entry.location).collect(),
            moving,
        );
        Ok(())
    }

    /// Restores the fill, or the focused item, after the usual confirmation.
    pub fn restore_targets(&self) -> TargetCommand {
        let entries = self.command_targets();
        if entries.is_empty() {
            return TargetCommand::Nothing;
        }
        if !entries.iter().all(|entry| is_trash_item(&entry.location)) {
            return TargetCommand::Refused;
        }
        self.state.request_restore(entries);
        TargetCommand::Started
    }

    pub fn compress_targets(&self) -> TargetCommand {
        let entries = self.command_targets();
        if entries.is_empty() {
            return TargetCommand::Nothing;
        }
        if entries
            .iter()
            .any(|entry| entry.location.native_path().is_none())
        {
            return TargetCommand::Refused;
        }
        self.state.show_compress_dialog(entries);
        TargetCommand::Started
    }

    /// The one archive the fill, or the focused item, names. `Err` explains why
    /// there is none.
    pub fn extract_target(&self) -> Result<FileEntry, &'static str> {
        let mut entries = self.command_targets();
        let entry = match entries.len() {
            0 => return Err("Nothing to extract"),
            1 => entries.remove(0),
            _ => return Err("Extract one archive at a time"),
        };
        if entry.is_directory()
            || entry.location.native_path().is_none()
            || ArchiveFormat::from_extension(&entry.display_name).is_none()
        {
            return Err("Not an archive");
        }
        Ok(entry)
    }

    pub fn extract_here(&self, entry: FileEntry) {
        self.state.extract_entry(entry);
    }

    /// Extracts into the typed folder and stays in the current one.
    pub fn extract_to_typed(&self, entry: FileEntry, text: &str) -> Result<(), &'static str> {
        let destination = self.typed_destination_folder(text)?;
        self.state
            .extract_entry_to(entry, Location::local(destination));
        Ok(())
    }
}
