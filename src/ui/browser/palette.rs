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

pub(in crate::ui) struct PaletteTarget {
    depth: Option<usize>,
    location: Option<Location>,
    entries: Vec<FileEntry>,
    search_results: bool,
    item_shortcuts: bool,
}

impl PaletteTarget {
    pub(in crate::ui) fn accepts_item_shortcuts(&self) -> bool {
        self.item_shortcuts
    }
}

impl BrowserView {
    pub(in crate::ui) fn capture_palette_target(&self) -> PaletteTarget {
        self.state.cancel_click_rename();
        self.state
            .input_ownership
            .borrow_mut()
            .keyboard_navigation();
        self.state.sync_mode_selection();
        self.state.refresh_destination_style();
        let depth = self.state.destination_depth();
        let search_entries = self.palette_search_entries();
        PaletteTarget {
            depth,
            location: depth.and_then(|depth| self.state.browser.location_at(depth)),
            search_results: search_entries.is_some(),
            item_shortcuts: self.item_view_has_focus() && search_entries.is_none(),
            entries: search_entries.unwrap_or_else(|| self.state.browser.selected_entries()),
        }
    }

    fn palette_search_entries(&self) -> Option<Vec<FileEntry>> {
        let filtered = self.view_mode() != BrowserMode::Columns
            || self.state.destination_depth().is_some_and(|depth| {
                self.state
                    .columns
                    .borrow()
                    .get(depth)
                    .is_some_and(|column| {
                        column.recursive_search_active.get() || column.map.has_query()
                    })
            });
        filtered.then(|| self.selected_search_results()).flatten()
    }

    pub(in crate::ui) fn palette_target_unavailable(
        &self,
        target: &PaletteTarget,
    ) -> Option<&'static str> {
        let location = target
            .depth
            .and_then(|depth| self.state.browser.location_at(depth));
        if target.location.is_none() {
            Some("Open a folder first")
        } else if location != target.location {
            Some("The originating folder changed; reopen the palette")
        } else {
            None
        }
    }

    pub(in crate::ui) fn palette_terminal_location(
        &self,
        target: &PaletteTarget,
    ) -> Option<Location> {
        selected_terminal_location(&target.entries)
            .or_else(|| target.location.clone())
            .filter(|location| location.native_path().is_some())
    }

    pub(in crate::ui) fn execute_palette_terminal(&self, target: &PaletteTarget) {
        if let Some(location) = self.palette_terminal_location(target) {
            launch_terminal(&location, &self.state.overlay);
        }
    }

    pub(in crate::ui) fn palette_pin_status(&self, target: &PaletteTarget) -> PinStatus {
        let [entry] = target.entries.as_slice() else {
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
        target: &PaletteTarget,
    ) -> Option<&'static str> {
        if command != FileCommand::Undo
            && let Some(reason) = self.palette_target_unavailable(target)
        {
            return Some(reason);
        }
        let entries = &target.entries;
        match command {
            FileCommand::NewFolder => match target.location.as_ref() {
                Some(location) if is_trash_location(location) => {
                    Some("Folders can't be created in Trash")
                }
                Some(location) if location.is_recent_location() => {
                    Some("Open a folder outside Recent to create a folder")
                }
                _ => None,
            },
            FileCommand::Undo => (!self.state.browser.can_undo()).then_some("Nothing to undo"),
            FileCommand::CopyPaths => entries.is_empty().then_some("Select an item first"),
            FileCommand::Duplicate => {
                if entries.is_empty() {
                    Some("Select an item first")
                } else {
                    duplicate_transfer(entries)
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
            FileCommand::Pin => (self.palette_pin_status(target) == PinStatus::Unavailable)
                .then_some("Select a pinnable folder"),
        }
    }

    pub(in crate::ui) fn execute_palette_file(&self, command: FileCommand, target: &PaletteTarget) {
        let entries = &target.entries;
        match command {
            FileCommand::NewFolder => {
                if let (Some(depth), Some(location)) = (target.depth, target.location.as_ref()) {
                    self.state.begin_new_entry(depth, location.clone(), true);
                }
            }
            FileCommand::Rename => {
                if let Some(depth) = target.depth {
                    let position = (!target.search_results)
                        .then(|| self.state.browser.column_snapshot(depth))
                        .flatten()
                        .and_then(|column| {
                            (0..column.count).find(|position| {
                                self.state
                                    .browser
                                    .entry_at(depth, *position)
                                    .is_some_and(|entry| entry.location == entries[0].location)
                            })
                        });
                    context_menu::rename_context_entry(
                        &self.state,
                        depth,
                        position,
                        entries[0].clone(),
                    );
                }
            }
            FileCommand::Duplicate => {
                if let Some((destination, sources)) = duplicate_transfer(entries) {
                    self.state.start_transfer(destination, sources, false);
                }
            }
            FileCommand::CopyPaths => copy_locations(entries),
            FileCommand::Properties => {
                if let Some(depth) = target.depth {
                    self.state
                        .show_entry_properties_at(entries[0].clone(), depth);
                }
            }
            FileCommand::Pin => {
                let entry = &entries[0];
                if self.palette_pin_status(target) == PinStatus::Pinned {
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
