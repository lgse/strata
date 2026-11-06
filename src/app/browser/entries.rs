// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use crate::{
    model::{FileEntry, Location, ViewPreferences},
    services::RequestId,
};

use super::{Browser, BrowserColumnSnapshot, ColumnEntryCounts};

impl Browser {
    pub fn entry_at(&self, depth: usize, position: usize) -> Option<FileEntry> {
        self.state.borrow().entry_at(depth, position)
    }

    pub(crate) fn resolve_entry_at(
        &self,
        depth: usize,
        position: usize,
        location: &Location,
    ) -> Option<(usize, FileEntry)> {
        self.state
            .borrow_mut()
            .resolve_entry_at(depth, position, location)
    }

    pub fn column_entry_counts(&self, depth: usize) -> Option<ColumnEntryCounts> {
        self.state.borrow().column_entry_counts(depth)
    }

    pub fn with_column_entries<R>(
        &self,
        depth: usize,
        read: impl FnOnce(&[FileEntry]) -> R,
    ) -> Option<R> {
        let state = self.state.borrow();
        Some(read(&state.columns.get(depth)?.entries))
    }

    pub fn with_entries<R>(
        &self,
        depth: usize,
        range: std::ops::Range<usize>,
        read: impl FnOnce(&[FileEntry]) -> R,
    ) -> Option<R> {
        let state = self.state.borrow();
        let entries = &state.columns.get(depth)?.entries;
        Some(read(entries.get(range)?))
    }

    pub fn column_preferences(&self, depth: usize) -> Option<ViewPreferences> {
        self.state.borrow().column_preferences(depth)
    }

    /// The presentation groups listings by file type, which orders the rows it shows
    /// and so where a load's automatic cursor lands.
    pub fn set_type_grouping(&self, grouped: bool) {
        self.state.borrow_mut().set_type_grouping(grouped);
    }

    pub fn shows_type_groups(&self, depth: usize) -> bool {
        self.state.borrow().shows_type_groups(depth)
    }

    pub(crate) fn column_request_id(&self, depth: usize) -> Option<RequestId> {
        self.state.borrow().request_id_for_depth(depth)
    }

    pub fn column_snapshot(&self, depth: usize) -> Option<BrowserColumnSnapshot> {
        let state = self.state.borrow();
        let column = state.columns.get(depth)?;
        Some(BrowserColumnSnapshot {
            location: column.location.clone(),
            count: column.entries.len(),
            selected_positions: state.selected_positions(depth),
            loading: column.load_state == crate::app::navigation::LoadState::Loading,
            error: match &column.load_state {
                crate::app::navigation::LoadState::Error(message)
                | crate::app::navigation::LoadState::Denied { message, .. } => {
                    Some(message.clone())
                }
                _ => None,
            },
            administrator: match &column.load_state {
                crate::app::navigation::LoadState::Denied { administrator, .. } => {
                    Some(administrator.clone())
                }
                _ => None,
            },
            truncated: column.truncated,
            reveal_pending: state.reveal_pending(depth),
            not_responding: self.column_not_responding(depth),
        })
    }

    pub fn focused_item(&self) -> Option<(usize, usize, FileEntry)> {
        self.state.borrow().focused_entry()
    }

    pub fn rename_item(&self) -> Option<(usize, usize, FileEntry)> {
        let state = self.state.borrow();
        state.focused_entry().or_else(|| state.entered_folder())
    }

    pub fn focused_entry(&self) -> Option<FileEntry> {
        self.focused_item().map(|(_, _, entry)| entry)
    }

    pub fn cursor_entry(&self, depth: usize) -> Option<FileEntry> {
        self.state.borrow().cursor_entry(depth)
    }

    pub(super) fn entry_at_location(&self, location: &Location) -> Option<FileEntry> {
        self.state
            .borrow()
            .columns
            .iter()
            .flat_map(|column| column.entries.iter())
            .find(|entry| &entry.location == location)
            .cloned()
    }

    pub fn selected_positions(&self, depth: usize) -> Vec<usize> {
        self.state.borrow().selected_positions(depth)
    }

    pub fn active_selection_contains(&self, depth: usize, position: usize) -> bool {
        self.state
            .borrow()
            .active_selection_contains(depth, position)
    }

    pub fn selected_entries(&self) -> Vec<FileEntry> {
        self.state.borrow().selected_entries()
    }

    pub fn command_entries(&self, depth: usize) -> Vec<FileEntry> {
        self.state.borrow().command_entries(depth)
    }

    pub fn filled_entries(&self, depth: usize) -> Vec<FileEntry> {
        self.state.borrow().filled_entries(depth)
    }

    pub fn selected_count(&self) -> usize {
        self.state.borrow().selected_count()
    }

    pub fn selection_is_load_cursor(&self) -> bool {
        self.state.borrow().selection_is_load_cursor()
    }

    pub fn commit_selection(&self) {
        self.state.borrow_mut().commit_selection();
    }

    pub fn deletion_entries(&self) -> Vec<FileEntry> {
        self.selection_or_descended()
    }

    /// Entries a copy/cut acts on: the active selection, or the folder that
    /// was descended into when the focused column selected nothing yet.
    pub fn transfer_entries(&self) -> Vec<FileEntry> {
        self.selection_or_descended()
    }

    pub(super) fn selection_or_descended(&self) -> Vec<FileEntry> {
        let state = self.state.borrow();
        let selected = state.selected_entries();
        if !selected.is_empty() {
            return selected;
        }
        state
            .entered_folder()
            .map(|(_, _, entry)| entry)
            .into_iter()
            .collect()
    }

    pub fn entries_named(&self, names: &HashSet<String>) -> Vec<FileEntry> {
        self.state
            .borrow()
            .columns
            .iter()
            .flat_map(|column| column.entries.iter())
            .filter(|entry| names.contains(&entry.display_name))
            .cloned()
            .collect()
    }

    pub fn active_child_position(&self, depth: usize) -> Option<usize> {
        self.state.borrow().active_child_position(depth)
    }
}
