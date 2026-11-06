// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet};

use crate::{
    model::{FileEntry, Location},
    services::RequestId,
};

use super::{
    ColumnState, EntryInsertion, LoadState, NavigationState, merge_entries, place_committed_cursor,
};

impl NavigationState {
    pub fn select_first_on_load(&mut self, depth: usize) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.select_first_on_load = true;
        }
    }

    /// Sets the soft load target (`preferred_on_load`) without clearing what the load restores.
    pub fn prefer_location_on_load(&mut self, depth: usize, location: Location) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.preferred_on_load = Some(location);
            column.select_first_on_load = false;
        }
    }

    pub fn select_locations_on_load(&mut self, depth: usize, targets: Vec<Location>) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.preferred_on_load = None;
            column.selected = None;
            column.selected_locations.clear();
            column.selection_anchor = None;
            column.selection_target = None;
            column.pending_selection.clear();
            column.pending_reveal = targets;
            column.selection_from_reveal = false;
            column.select_first_on_load = false;
            column.load_cursor = None;
        }
    }

    pub fn reveal_pending(&self, depth: usize) -> bool {
        self.columns
            .get(depth)
            .is_some_and(|column| !column.pending_reveal.is_empty())
    }

    /// Consume the focus request so later publications cannot steal focus again.
    pub fn take_selection_from_reveal(&mut self, depth: usize) -> bool {
        self.columns
            .get_mut(depth)
            .is_some_and(|column| std::mem::take(&mut column.selection_from_reveal))
    }

    pub fn take_resolved_location_reveal(
        &mut self,
        depth: usize,
        request_id: RequestId,
    ) -> Option<bool> {
        let column = self.columns.get_mut(depth)?;
        if column.request_id != request_id {
            return None;
        }
        let requested: HashSet<&Location> = column.pending_reveal.iter().collect();
        let (mut listed, mut any_hidden) = (0, false);
        for entry in column
            .entries
            .iter()
            .filter(|entry| requested.contains(&entry.location))
        {
            listed += 1;
            any_hidden |= entry.is_hidden;
        }
        if listed == 0 {
            return None;
        }
        if listed == requested.len() {
            column.pending_reveal.clear();
        }
        Some(any_hidden)
    }

    pub fn lists_every_target(&self, depth: usize, targets: &[Location]) -> bool {
        let requested: HashSet<&Location> = targets.iter().collect();
        self.columns.get(depth).is_some_and(|column| {
            column
                .entries
                .iter()
                .filter(|entry| requested.contains(&entry.location))
                .count()
                == requested.len()
        })
    }

    pub fn take_unresolved_location_reveal(
        &mut self,
        depth: usize,
        request_id: RequestId,
    ) -> Option<Location> {
        let column = self.columns.get_mut(depth)?;
        if column.request_id != request_id {
            return None;
        }
        let targets = std::mem::take(&mut column.pending_reveal);
        let requested: HashSet<&Location> = targets.iter().collect();
        if column
            .entries
            .iter()
            .any(|entry| requested.contains(&entry.location))
        {
            return None;
        }
        targets.into_iter().next()
    }

    pub fn apply_batch(
        &mut self,
        request_id: RequestId,
        entries: Vec<FileEntry>,
    ) -> Option<(usize, Vec<EntryInsertion>)> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        let preferences = column.preferences;
        let selected_location = column.cursor_location();
        // Spares a scan in the selection read that follows each batch.
        let lone_cursor = column.selected_locations.lone_position() == column.selected
            && selected_location
                .as_ref()
                .is_some_and(|location| entries.iter().all(|entry| &entry.location != location));
        column.exclude_new_from_full_fill(&entries);
        let (merged, insertions) =
            merge_entries(std::mem::take(&mut column.entries), entries, preferences);
        column.entries = merged;
        column.invalidate_entry_indexes();
        if let Some(selected_location) =
            selected_location.or_else(|| column.selection_target.clone())
        {
            column.select_target(&selected_location);
        }
        if lone_cursor && let Some(cursor) = column.selected {
            column.selected_locations.move_lone_position(cursor);
        }
        column.resolve_load_intents();
        Some((depth, insertions))
    }

    /// Entries must arrive pre-sorted; the caller marks the load finished and publishes.
    pub fn install_snapshot(
        &mut self,
        request_id: RequestId,
        entries: Vec<FileEntry>,
    ) -> Option<usize> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        column.entries = entries;
        column.invalidate_entry_indexes();
        if let Some(selected_location) = column.selection_target.clone() {
            column.select_target(&selected_location);
        }
        column.resolve_load_intents();
        // The listing is complete, and its selection is published before `finish`.
        column.complete_load_selection();
        Some(depth)
    }

    pub fn open_load_depth(&self, request_id: RequestId) -> Option<usize> {
        self.columns.iter().enumerate().find_map(|(depth, column)| {
            (column.request_id == request_id && column.load_state == LoadState::Loading)
                .then_some(depth)
        })
    }

    /// Returns the finished depth and whether completing the listing moved the cursor,
    /// which batch loads have not published yet.
    pub fn finish(
        &mut self,
        request_id: RequestId,
        truncated: bool,
        can_trash: Option<bool>,
        can_delete: Option<bool>,
    ) -> Option<(usize, bool)> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        let cursor_moved = column.complete_load_selection();
        column.select_first_on_load = false;
        column.retain_selected_locations(false);
        column.pending_selection.clear();
        column.truncated = truncated;
        column.can_trash = can_trash;
        column.can_delete = can_delete;
        column.settle_listed_state();
        Some((depth, cursor_moved))
    }

    pub fn keep_listing(&mut self, request_id: RequestId, truncated: bool) -> bool {
        let Some((_, column)) = self.column_for_request_mut(request_id) else {
            return false;
        };
        column.pending_selection.clear();
        column.open_child_on_load = None;
        column.truncated = truncated;
        column.settle_listed_state();
        true
    }

    pub fn fail(
        &mut self,
        request_id: RequestId,
        message: String,
        administrator: Option<Location>,
    ) -> Option<usize> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        column.drop_load_intents();
        column.load_state = match administrator {
            Some(administrator) => LoadState::Denied {
                message,
                administrator,
            },
            None => LoadState::Error(message),
        };
        Some(depth)
    }
}

impl ColumnState {
    /// A full fill (Select All) must not grow to cover newly listed entries.
    fn exclude_new_from_full_fill(&mut self, entries: &[FileEntry]) {
        if !self.selected_locations.is_all() {
            return;
        }
        let existing: HashSet<_> = self.entries.iter().map(|entry| &entry.location).collect();
        for entry in entries
            .iter()
            .filter(|entry| !existing.contains(&entry.location))
        {
            self.selected_locations.excludes_new(entry.location.clone());
        }
    }

    /// Puts the cursor on `location` once it is listed, settling the target.
    fn select_target(&mut self, location: &Location) {
        self.selected = self.position_of(location);
        if self.selected.is_some() {
            self.selection_target = None;
        }
    }

    /// Applies what the load meant to select to the entries listed so far.
    fn resolve_load_intents(&mut self) {
        self.resolve_pending_reveal();
        self.restore_pending_selection();
        self.resolve_preferred_on_load();
        self.resolve_open_child_on_load();
        if self.picks_first_row_from_batches() {
            self.select_first_displayed_entry();
        }
    }

    /// A grouped listing's first row is known only once every batch is in.
    fn picks_first_row_from_batches(&self) -> bool {
        self.select_first_on_load && !self.entries.is_empty() && !self.shows_type_groups()
    }

    /// Nothing selected or revealed the cursor in a listing that has entries.
    fn awaits_fallback_cursor(&self) -> bool {
        self.selected.is_none() && self.pending_reveal.is_empty() && !self.entries.is_empty()
    }

    /// A user selection or a failed load supersedes whatever the load meant to select.
    pub(super) fn drop_load_intents(&mut self) {
        self.pending_reveal.clear();
        self.selection_from_reveal = false;
        self.preferred_on_load = None;
        self.open_child_on_load = None;
        self.reload_cursor = None;
    }

    /// Selects `position` as a load's automatic choice: cursor, single selection and
    /// anchor, marked so a later cursor move does not carry it along.
    pub(super) fn select_loaded_entry(&mut self, position: usize) {
        let location = self.entries[position].location.clone();
        self.selected = Some(position);
        self.selected_locations.clear();
        self.selected_locations.insert(location.clone());
        self.selection_anchor = Some(location.clone());
        self.select_first_on_load = false;
        self.load_cursor = Some(location);
    }

    pub(super) fn select_first_displayed_entry(&mut self) {
        self.pending_selection.clear();
        if let Some(position) = self.first_displayed_entry() {
            self.select_loaded_entry(position);
        }
    }

    /// An explicit target, or a selection the load already restored, wins over the
    /// preferred entry.
    pub(super) fn resolve_preferred_on_load(&mut self) {
        if !self.pending_reveal.is_empty() || self.selected.is_some() {
            return;
        }
        let Some(preferred) = self.preferred_on_load.as_ref() else {
            return;
        };
        let show_hidden = self.preferences.show_hidden;
        let Some(position) = self
            .entries
            .iter()
            .position(|entry| entry.location == *preferred && (show_hidden || !entry.is_hidden))
        else {
            return;
        };
        self.preferred_on_load = None;
        self.pending_selection.clear();
        self.select_loaded_entry(position);
    }

    /// Unlike the preferred entry, the open folder has no fallback: a cursor on
    /// another entry would make file commands act on it.
    fn resolve_open_child_on_load(&mut self) {
        if !self.pending_reveal.is_empty() || self.selected.is_some() {
            return;
        }
        let Some(open_child) = self.open_child_on_load.as_ref() else {
            return;
        };
        let Some(position) = self
            .entries
            .iter()
            .position(|entry| entry.location == *open_child && self.shows(entry))
        else {
            return;
        };
        self.pending_selection.clear();
        self.select_first_on_load = false;
        place_committed_cursor(self, position);
    }

    /// Picks that need the complete listing: the first row shown for a grouped first
    /// visit or a preferred entry that never arrived, and a cursor-only neighbour for a
    /// reloaded cursor whose entry is gone. Returns whether the cursor moved.
    pub(super) fn complete_load_selection(&mut self) -> bool {
        let before = self.selected;
        self.open_child_on_load = None;
        let wants_first_row = self.preferred_on_load.take().is_some() || self.select_first_on_load;
        if wants_first_row && self.awaits_fallback_cursor() {
            self.select_first_displayed_entry();
        }
        if let Some(previous) = self.reload_cursor.take()
            && self.awaits_fallback_cursor()
        {
            self.selected = self.visible_neighbor(previous.min(self.entries.len() - 1));
            if self.selected.is_some() {
                self.selection_target = None;
            }
        }
        self.selected != before
    }

    pub(super) fn resolve_pending_reveal(&mut self) {
        if self.pending_reveal.is_empty() {
            return;
        }
        let requested: HashSet<&Location> = self.pending_reveal.iter().collect();
        let listed: HashMap<&Location, usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| requested.contains(&entry.location))
            .map(|(position, entry)| (&entry.location, position))
            .collect();
        let Some((position, first)) = self
            .pending_reveal
            .iter()
            .find_map(|target| Some((*listed.get(target)?, target.clone())))
        else {
            return;
        };
        let selected: HashSet<Location> = listed.into_keys().cloned().collect();
        self.selected = Some(position);
        self.selected_locations = selected.into();
        self.selection_anchor = Some(first);
        self.selection_target = None;
        self.load_cursor = None;
        self.selection_from_reveal = true;
        self.preferred_on_load = None;
        self.open_child_on_load = None;
        self.reload_cursor = None;
    }

    pub(super) fn restore_pending_selection(&mut self) {
        if self.pending_selection.is_empty() {
            return;
        }
        let restored: HashSet<_> = self
            .entries
            .iter()
            .filter(|entry| self.pending_selection.contains(&entry.location))
            .map(|entry| entry.location.clone())
            .collect();
        if restored.is_empty() {
            return;
        }
        if !self
            .selected_locations
            .equals_explicit(&restored, &self.entries)
        {
            self.selected_locations = restored.into();
        }
        if self.selected.is_none() {
            self.selected = self
                .entries
                .iter()
                .position(|entry| self.selected_locations.contains(&entry.location));
        }
    }
}
