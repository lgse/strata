// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::ui) enum FileCommand {
    NewFolder,
    Rename,
    Duplicate,
    CopyPaths,
    Properties,
    Pin,
    Undo,
}

impl BrowserView {
    pub(in crate::ui) fn prepare_palette(&self) {
        self.state.cancel_click_rename();
        self.state
            .input_ownership
            .borrow_mut()
            .keyboard_navigation();
        self.state.sync_mode_selection();
        self.state.refresh_destination_style();
    }

    fn palette_search_entries(&self) -> Option<Vec<FileEntry>> {
        let filtered = self.view_mode() != BrowserMode::Columns
            || self.state.destination_depth().is_some_and(|depth| {
                self.state
                    .columns
                    .borrow()
                    .get(depth)
                    .is_some_and(|column| {
                        column.search_handle.borrow().is_some() || column.map.has_query()
                    })
            });
        filtered.then(|| self.selected_search_results()).flatten()
    }

    fn palette_entries(&self) -> Vec<FileEntry> {
        if let Some(entries) = self.palette_search_entries() {
            return entries;
        }
        self.state.sync_mode_selection();
        self.state.browser.selected_entries()
    }

    pub(in crate::ui) fn palette_terminal_location(&self) -> Option<Location> {
        selected_terminal_location(&self.palette_entries())
            .or_else(|| {
                self.state
                    .destination_depth()
                    .and_then(|depth| self.state.browser.location_at(depth))
            })
            .filter(|location| location.native_path().is_some())
    }

    pub(in crate::ui) fn execute_palette_terminal(&self) {
        if let Some(location) = self.palette_terminal_location() {
            launch_terminal(&location, &self.state.overlay);
        }
    }

    pub(in crate::ui) fn palette_pin_status(&self) -> PinStatus {
        let entries = self.palette_entries();
        let [entry] = entries.as_slice() else {
            return PinStatus::Unavailable;
        };
        if !entry.is_directory() || is_trash_location(&entry.location) {
            return PinStatus::Unavailable;
        }
        self.state
            .pin_status_handler
            .borrow()
            .as_ref()
            .map_or(PinStatus::Unavailable, |handler| handler(&entry.location))
    }

    pub(in crate::ui) fn palette_file_unavailable(
        &self,
        command: FileCommand,
    ) -> Option<&'static str> {
        let entries = self.palette_entries();
        match command {
            FileCommand::NewFolder => {
                let location = self
                    .state
                    .destination_depth()
                    .and_then(|depth| self.state.browser.location_at(depth));
                (!location.is_some_and(|location| {
                    !is_trash_location(&location) && !location.is_recent_location()
                }))
                .then_some("Open a folder first")
            }
            FileCommand::Undo => (!self.state.browser.can_undo()).then_some("Nothing to undo"),
            FileCommand::CopyPaths => entries.is_empty().then_some("Select an item first"),
            FileCommand::Duplicate => {
                if entries.is_empty() {
                    Some("Select an item first")
                } else {
                    duplicate_transfer(&entries)
                        .is_none()
                        .then_some("Select items in the same folder outside Trash")
                }
            }
            FileCommand::Rename | FileCommand::Properties => {
                if entries.len() != 1 {
                    Some("Select one item")
                } else if command == FileCommand::Rename && is_trash_location(&entries[0].location)
                {
                    Some("Restore the item from Trash first")
                } else if command == FileCommand::Rename && self.state.rename_operation_pending() {
                    Some("Wait for the current rename to finish")
                } else {
                    None
                }
            }
            FileCommand::Pin => (self.palette_pin_status() == PinStatus::Unavailable)
                .then_some("Select a pinnable folder"),
        }
    }

    pub(in crate::ui) fn execute_palette_file(&self, command: FileCommand) {
        if self.palette_file_unavailable(command).is_some() {
            return;
        }
        let entries = self.palette_entries();
        match command {
            FileCommand::NewFolder => self.create_new_folder(),
            FileCommand::Rename => {
                if self.palette_search_entries().is_some() {
                    context_menu::rename_context_entry(
                        &self.state,
                        self.state.destination_depth().unwrap_or(0),
                        None,
                        entries[0].clone(),
                    );
                } else {
                    self.state.begin_rename();
                }
            }
            FileCommand::Duplicate => {
                if let Some((destination, sources)) = duplicate_transfer(&entries) {
                    self.state.start_transfer(destination, sources, false);
                }
            }
            FileCommand::CopyPaths => copy_locations(&entries),
            FileCommand::Properties => self.state.show_entry_properties(entries[0].clone()),
            FileCommand::Pin => {
                let entry = &entries[0];
                if self.palette_pin_status() == PinStatus::Pinned {
                    if let Some(handler) = self.state.unpin_handler.borrow().as_ref() {
                        handler(&entry.location);
                    }
                } else if let Some(handler) = self.state.pin_handler.borrow().as_ref() {
                    handler(entry.location.clone(), entry.display_name.clone());
                }
            }
            FileCommand::Undo => {
                self.undo_last_operation();
            }
        }
    }
}
