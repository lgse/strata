// SPDX-License-Identifier: MIT

//! Renaming, clipboard, file commands and undo for a browser view.

use super::*;

fn restored_placeholder(location: &Location) -> FileEntry {
    let display_name = location
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    FileEntry {
        location: location.clone(),
        native_name: location.file_name().unwrap_or_default().to_os_string(),
        thumbnail_path: None,
        display_name,
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
        listable: crate::model::MetadataValue::Unknown,
        recent_uri: None,
    }
}

impl BrowserView {
    pub fn begin_rename(&self) -> bool {
        if self.filter_has_focus() || self.selected_search_results().is_some() {
            let Some(entry) = self.selected_search_result() else {
                return false;
            };
            if self.state.rename_operation_pending() {
                return false;
            }
            self.state.cancel_new_entry();
            context_menu::rename_context_entry(
                &self.state,
                self.state.destination_depth().unwrap_or(0),
                None,
                entry,
            );
            return true;
        }
        self.state.begin_rename()
    }

    pub fn cancel_rename(&self) -> bool {
        self.state.cancel_rename()
    }

    /// Tab inside the focused inline rename: commits it and renames the neighbouring item.
    pub fn advance_rename(&self, direction: gtk::DirectionType) -> bool {
        let Some(field) = self.active_rename_field() else {
            return false;
        };
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        if !focused.is_some_and(|focused| focused == field || focused.is_ancestor(&field)) {
            return false;
        }
        let step = if direction == gtk::DirectionType::TabBackward {
            -1
        } else {
            1
        };
        self.state.advance_rename(step)
    }

    pub fn cancel_new_entry(&self) -> bool {
        self.state.cancel_new_entry()
    }

    pub fn rename_is_active(&self) -> bool {
        self.state.active_rename.borrow().is_some()
            || self.state.mode_views.borrow().rename_is_active()
    }

    pub(in crate::ui) fn cancel_pending_click_rename(&self) {
        self.state.cancel_click_rename();
    }

    pub fn active_rename_field(&self) -> Option<gtk::Entry> {
        self.state
            .active_rename
            .borrow()
            .as_ref()
            .map(|rename| rename.field.clone())
            .or_else(|| self.state.mode_views.borrow().active_rename_field())
    }

    pub fn new_entry_is_active(&self) -> bool {
        self.state.pending_new_entry.borrow().is_some()
    }

    pub fn inline_edit_is_open(&self) -> bool {
        self.state.inline_edit_is_open()
    }

    pub fn create_new_folder(&self) {
        if let Some((depth, location)) = self.new_entry_parent() {
            self.state.begin_new_entry(depth, location, true);
        }
    }

    pub(super) fn new_entry_parent(&self) -> Option<(usize, Location)> {
        let depth = if self.view_mode() == BrowserMode::Columns {
            new_folder_destination_depth(
                self.state.focused_column_depth(),
                self.state.browser.active_depth(),
                self.state.columns.borrow().len(),
            )
        } else {
            self.state.browser.active_depth()
        }?;
        self.state
            .browser
            .location_at(depth)
            .map(|location| (depth, location))
    }

    pub fn paste(&self) {
        if let Some(location) = self.paste_location() {
            self.state.paste_into(location);
        }
    }

    pub(super) fn paste_location(&self) -> Option<Location> {
        self.state.sync_mode_selection();
        let selected = if self.state.browser.selected_count() == 1 {
            self.state.browser.selected_entries()
        } else {
            Vec::new()
        };
        let column = self
            .state
            .destination_depth()
            .and_then(|depth| self.state.browser.location_at(depth));
        paste_destination(
            &selected,
            column,
            self.state.browser.selection_is_load_cursor(),
        )
    }

    /// The selected search hits, else the mode's synced transfer selection.
    fn clipboard_entries(&self) -> Vec<FileEntry> {
        self.selected_search_results().unwrap_or_else(|| {
            self.state.sync_mode_selection();
            self.state.browser.transfer_entries()
        })
    }

    pub fn copy_selection(&self) -> bool {
        let entries = self.clipboard_entries();
        if entries.is_empty() {
            return false;
        }
        self.state.copy_entries(&entries);
        true
    }

    pub fn duplicate_selection(&self) -> bool {
        self.state.sync_mode_selection();
        self.duplicate_entries(&self.state.browser.transfer_entries())
    }

    pub(in crate::ui) fn duplicate_entries(&self, entries: &[FileEntry]) -> bool {
        let Some((destination, sources)) = duplicate_transfer(entries) else {
            return false;
        };
        self.state.start_transfer(destination, sources, false);
        true
    }

    pub fn cut_selection(&self) -> bool {
        let entries = self.clipboard_entries();
        if entries.is_empty() {
            return false;
        }
        self.state.cut_entries(&entries)
    }

    pub fn copy_path(&self) -> bool {
        self.state.sync_mode_selection();
        let entries = self.state.browser.selected_entries();
        if entries.is_empty() {
            let Some(entry) = self.state.browser.focused_entry() else {
                return false;
            };
            copy_locations(&[entry]);
        } else {
            copy_locations(&entries);
        }
        true
    }

    pub fn pin_focused(&self) {
        self.state.sync_mode_selection();
        if let Some(entry) = self.state.browser.focused_entry() {
            self.pin_entry(entry);
        }
    }

    pub(in crate::ui) fn pin_entry(&self, entry: FileEntry) {
        let status = self
            .state
            .pin_status_handler
            .borrow()
            .as_ref()
            .map_or(PinStatus::Unavailable, |handler| handler(&entry.location));
        if !can_pin_entry(&entry, status) {
            return;
        }
        if let Some(handler) = self.state.pin_handler.borrow().as_ref() {
            handler(entry.location, entry.display_name, Box::new(|_| {}));
        }
    }

    pub fn open_terminal(&self) {
        self.state.sync_mode_selection();
        let selected = self.state.browser.selected_entries();
        let location = selected_terminal_location(&selected).or_else(|| {
            if self.view_mode() != BrowserMode::Columns {
                let depth = self.state.browser.active_depth()?;
                return self.state.browser.location_at(depth);
            }
            let depth = self.state.destination_depth()?;
            let directory = self.state.browser.location_at(depth);
            // A click opens the folder as the active column while the pointer can
            // leave its parent as the destination; prefer the folder it opened.
            let opened = self.state.browser.location_at(depth + 1);
            let selected_in_destination: Vec<_> = self
                .state
                .browser
                .selected_positions(depth)
                .into_iter()
                .filter_map(|position| self.state.browser.entry_at(depth, position))
                .collect();
            match selected_terminal_location(&selected_in_destination) {
                Some(selected)
                    if self.state.browser.active_depth() == Some(depth + 1)
                        && opened.as_ref() == Some(&selected) =>
                {
                    Some(selected)
                }
                _ => directory,
            }
        });
        let Some(location) = location else {
            return;
        };
        launch_terminal(&location, &self.state.overlay);
    }

    /// A user Reload also retries Trash on mounts that refused it earlier.
    pub fn refresh(&self) {
        crate::adapters::trash_support::forget_all();
        self.state.refresh_browser();
    }

    pub fn show_location_properties(&self, location: &Location) {
        self.state.show_folder_properties(location);
    }

    pub fn show_focused_properties(&self) -> bool {
        if let Some(selected) = self.selected_search_results() {
            if selected.is_empty() {
                return false;
            }
            self.state.show_selection_properties(selected);
            return true;
        }
        self.state.sync_mode_selection();
        let selected = self.state.browser.selected_entries();
        if selected.len() > 1 {
            self.state.show_selection_properties(selected);
            return true;
        }
        let Some(entry) = self.state.browser.focused_entry() else {
            return false;
        };
        self.state.show_entry_properties(entry);
        true
    }

    pub fn confirm_empty_trash(&self) {
        self.state.load_trash_summary();
    }

    pub fn confirm_delete(&self, permanent: bool) -> bool {
        self.state.sync_mode_selection();
        let entries = if let Some(entries) = self.selected_search_results() {
            entries
        } else if self.view_mode() == BrowserMode::Columns {
            self.focused_listing_depth()
                .map(|depth| self.state.browser.command_entries(depth))
                .filter(|entries| !entries.is_empty())
                .unwrap_or_else(|| self.state.browser.deletion_entries())
        } else {
            self.state.browser.deletion_entries()
        };
        if entries.is_empty() {
            return false;
        }
        let in_trash = self.focused_location_is_trash();
        self.state.request_delete(entries, permanent || in_trash);
        true
    }

    pub(super) fn focused_location_is_trash(&self) -> bool {
        self.state
            .focused_column_depth()
            .and_then(|depth| self.state.browser.location_at(depth))
            .or_else(|| self.state.browser.active_location())
            .as_ref()
            .is_some_and(is_trash_location)
    }

    /// Undoes the newest pending non-trash operation, if there is one.
    fn undo_pending_operation(&self) -> Option<bool> {
        let browser = &self.state.browser;
        if let Some((generation, _, _)) = browser.pending_undo_rename() {
            return Some(browser.undo_rename(generation));
        }
        if let Some((generation, records)) = browser.pending_undo_move() {
            return Some(self.state.undo_move(generation, records));
        }
        if let Some((generation, records)) = browser.pending_undo_group() {
            return Some(self.state.undo_group(generation, records));
        }
        if let Some((generation, locations)) = browser.pending_undo_copy() {
            return Some(self.state.undo_copy(generation, locations));
        }
        if let Some((generation, created, overwritten)) = browser.pending_undo_merge() {
            return Some(self.state.undo_merge(generation, created, overwritten));
        }
        None
    }

    /// The entries a trash undo restores, for its fly-out animation: the listed trash
    /// items while the trash is open, else placeholders at the restored locations.
    fn untrashed_entries(&self, locations: &[Location]) -> Vec<FileEntry> {
        let is_trash = self
            .state
            .browser
            .active_location()
            .as_ref()
            .is_some_and(paths::is_trash_location);
        if is_trash {
            let names: HashSet<String> = locations
                .iter()
                .filter_map(|location| location.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect();
            return self.state.browser.entries_named(&names);
        }
        locations.iter().map(restored_placeholder).collect()
    }

    pub fn undo_last_operation(&self) -> bool {
        if let Some(undone) = self.undo_pending_operation() {
            return undone;
        }
        let Some(locations) = self.state.browser.pending_undo_trash() else {
            return self.state.browser.undo_last_trash();
        };
        let entries = self.untrashed_entries(&locations);
        let source = self.state.delete_animation_source();
        let trash_button = self.state.trash_button.upgrade();
        let animation = source.zip(trash_button).and_then(|(source, trash_button)| {
            fly_to_trash::prepare_fly_from_trash(&source, entries.iter(), &trash_button)
        });
        self.state
            .pending_file_operation_animation
            .replace(animation);
        let undone = self.state.browser.undo_last_trash();
        if !undone {
            self.state.pending_file_operation_animation.take();
        }
        undone
    }

    pub fn redo_last_operation(&self) -> bool {
        if let Some((generation, _, _)) = self.state.browser.pending_redo_rename() {
            return self.state.browser.redo_rename(generation);
        }
        if let Some((generation, records)) = self.state.browser.pending_redo_move() {
            return self.state.redo_move(generation, records);
        }
        if let Some((generation, locations)) = self.state.browser.pending_redo_copy() {
            return self.state.browser.redo_copy(generation, locations);
        }
        if let Some((generation, locations)) = self.state.browser.pending_redo_trash() {
            return self.state.redo_trash(generation, locations);
        }
        false
    }
}
