// SPDX-License-Identifier: MIT

use std::{cmp::Ordering, collections::HashSet};

use crate::{
    model::{FileEntry, Location, MetadataValue, SortKey, ViewPreferences},
    services::{DirectoryChange, RequestId},
};

use super::{
    ColumnState, EntrySplice, EntrySpliceApplication, LoadState, NavigationState, compare_entries,
    merge_entries,
};

impl NavigationState {
    pub fn set_preserve_fill_on_removal(&mut self, preserve: bool) {
        self.preserve_fill_on_removal = preserve;
    }

    pub fn set_selectionless_removals(&mut self, locations: impl IntoIterator<Item = Location>) {
        self.selectionless_removals = locations.into_iter().collect();
    }

    pub fn retain_selectionless_removals(&mut self, locations: impl IntoIterator<Item = Location>) {
        let locations: HashSet<_> = locations.into_iter().collect();
        self.selectionless_removals
            .retain(|location| locations.contains(location));
    }

    pub fn apply_new_entries_batch(
        &mut self,
        depth: usize,
        watched: &Location,
        entries: Vec<FileEntry>,
    ) -> Result<EntrySpliceApplication, ()> {
        let Some(column) = self
            .columns
            .get_mut(depth)
            .filter(|column| &column.location == watched)
        else {
            return Err(());
        };
        if entries.is_empty() {
            return Ok(None);
        }
        let incoming_locations: HashSet<_> =
            entries.iter().map(|entry| entry.location.clone()).collect();
        if incoming_locations.len() != entries.len()
            || column
                .entries
                .iter()
                .any(|entry| incoming_locations.contains(&entry.location))
        {
            return Err(());
        }

        let selected_location = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        for entry in &entries {
            column
                .selected_locations
                .excludes_new(entry.location.clone());
        }
        let (merged, insertions) = merge_entries(
            std::mem::take(&mut column.entries),
            entries,
            column.preferences,
        );
        column.entries = merged;
        column.selected = selected_location.and_then(|location| {
            column
                .entries
                .iter()
                .position(|entry| entry.location == location)
        });
        column.invalidate_spliced_entry_indexes();
        column.load_state = LoadState::Ready;
        Ok(Some((
            insertions
                .into_iter()
                .map(|insertion| EntrySplice {
                    position: insertion.position,
                    removed: 0,
                    entries: insertion.entries,
                })
                .collect(),
            column.selected,
        )))
    }

    pub fn apply_removals_batch(
        &mut self,
        depth: usize,
        watched: &Location,
        locations: impl IntoIterator<Item = Location>,
    ) -> EntrySpliceApplication {
        let column = self
            .columns
            .get_mut(depth)
            .filter(|column| &column.location == watched)?;
        let removals: HashSet<_> = locations.into_iter().collect();
        if removals.is_empty() {
            return None;
        }

        let selected_location = column.cursor_location();
        let removed = RemovedEntries::split(
            std::mem::take(&mut column.entries),
            &removals,
            column.selected,
        );
        column.entries = removed.retained;
        if removed.locations.is_empty() {
            return None;
        }

        let selected_was_removed = selected_location
            .as_ref()
            .is_some_and(|location| removals.contains(location));
        let replace_selection = forget_removed(
            &mut self.selectionless_removals,
            column,
            &removed.locations,
            selected_location.as_ref(),
        );
        column.selected = match (selected_was_removed, replace_selection) {
            (true, true) => column.neighbour_after_removal(
                removed.retained_before_selected,
                self.preserve_fill_on_removal,
            ),
            (true, false) => None,
            (false, _) => selected_location.and_then(|location| column.position_of(&location)),
        };
        column.invalidate_spliced_entry_indexes();
        column.settle_listed_state();
        Some((removed.splices, column.selected))
    }

    pub fn apply_directory_change(
        &mut self,
        depth: usize,
        watched: &Location,
        change: DirectoryChange,
    ) -> EntrySpliceApplication {
        if !self
            .columns
            .get(depth)
            .is_some_and(|column| &column.location == watched)
        {
            return None;
        }
        let replace_selection = match &change {
            DirectoryChange::Remove(location) => !self.selectionless_removals.remove(location),
            _ => true,
        };
        let preserve_fill = self.preserve_fill_on_removal;
        let column = self.columns.get_mut(depth).expect("column checked");
        let mut monitored = MonitoredChange::new(column, preserve_fill);
        match change {
            DirectoryChange::Upsert(entry) => {
                if !monitored.upsert(entry) {
                    return None;
                }
            }
            DirectoryChange::Remove(location) => monitored.remove(&location, replace_selection),
            DirectoryChange::Move { from, entry } => monitored.relocate(&from, entry),
            DirectoryChange::Rescan => return None,
        }
        monitored.finish()
    }

    pub fn refresh_column(&mut self, depth: usize, request_id: RequestId) -> Option<Location> {
        let column = self.columns.get_mut(depth)?;
        column.selection_target = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone())
            .or_else(|| column.selection_target.clone());
        column.pending_selection = column.selected_locations.materialize(&column.entries);
        column.selected_locations = column.pending_selection.clone().into();
        column.load_state = LoadState::Loading;
        column.truncated = false;
        column.request_id = request_id;
        Some(column.location.clone())
    }

    pub fn reload_column(&mut self, depth: usize, request_id: RequestId) -> Option<Location> {
        // An open child column belongs to the cursor's folder; another entry must not
        // take over the cursor beside it.
        let child_open = depth + 1 < self.columns.len();
        let column = self.columns.get_mut(depth)?;
        column.selection_target = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone())
            .or_else(|| column.selection_target.clone());
        column.pending_selection = column.selected_locations.materialize(&column.entries);
        column.selected_locations = column.pending_selection.clone().into();
        column.entries = Vec::new();
        column.invalidate_entry_indexes();
        // A reload that restarts before the previous one listed anything keeps its cursor.
        column.reload_cursor = (!child_open)
            .then(|| column.selected.or(column.reload_cursor))
            .flatten();
        column.selected = None;
        column.load_state = LoadState::Loading;
        column.truncated = false;
        column.can_trash = None;
        column.can_delete = None;
        column.request_id = request_id;
        Some(column.location.clone())
    }

    pub fn relocate_column(&mut self, depth: usize, location: Location, request_id: RequestId) {
        let Some(previous) = self.reload_column(depth, request_id) else {
            return;
        };
        let column = &mut self.columns[depth];
        column.selected_locations = column
            .pending_selection
            .iter()
            .filter_map(|selected| selected.rebase(&previous, &location))
            .collect::<HashSet<_>>()
            .into();
        column.pending_selection = column
            .pending_selection
            .iter()
            .filter_map(|selected| selected.rebase(&previous, &location))
            .collect();
        for target in [
            &mut column.selection_target,
            &mut column.selection_anchor,
            &mut column.load_cursor,
            &mut column.open_child_on_load,
        ] {
            *target = target
                .as_ref()
                .and_then(|target| target.rebase(&previous, &location));
        }
        column.pending_reveal = column
            .pending_reveal
            .iter()
            .filter_map(|target| target.rebase(&previous, &location))
            .collect();
        column.location = location;
    }
}

pub(super) fn remove_monitored_entry(
    entries: &mut Vec<FileEntry>,
    location: &Location,
    splices: &mut Vec<EntrySplice>,
) {
    if let Some(position) = entries.iter().position(|entry| &entry.location == location) {
        entries.remove(position);
        splices.push(EntrySplice {
            position,
            removed: 1,
            entries: Vec::new(),
        });
    }
}

pub(super) fn upsert_monitored_entry(
    entries: &mut Vec<FileEntry>,
    entry: FileEntry,
    preferences: ViewPreferences,
    splices: &mut Vec<EntrySplice>,
) {
    if let Some(existing_position) = entries.iter().position(|e| e.location == entry.location) {
        let is_same_position = {
            let left_ok = existing_position == 0
                || compare_entries(&entries[existing_position - 1], &entry, preferences)
                    != Ordering::Greater;
            let right_ok = existing_position + 1 >= entries.len()
                || compare_entries(&entry, &entries[existing_position + 1], preferences)
                    != Ordering::Greater;
            left_ok && right_ok
        };

        if is_same_position {
            entries[existing_position] = entry.clone();
            splices.push(EntrySplice {
                position: existing_position,
                removed: 1,
                entries: vec![entry],
            });
            return;
        }

        remove_monitored_entry(entries, &entry.location, splices);
    }

    insert_monitored_entry(entries, entry, preferences, splices);
}

pub(super) fn insert_monitored_entry(
    entries: &mut Vec<FileEntry>,
    entry: FileEntry,
    preferences: ViewPreferences,
    splices: &mut Vec<EntrySplice>,
) {
    let position = if preferences.sort_key == SortKey::DeviceOrder {
        entries.len()
    } else if preferences.sort_key == SortKey::Unsorted {
        entries.partition_point(|current| {
            compare_entries(current, &entry, preferences) != std::cmp::Ordering::Greater
        })
    } else {
        entries
            .binary_search_by(|current| compare_entries(current, &entry, preferences))
            .unwrap_or_else(|position| position)
    };
    entries.insert(position, entry.clone());
    splices.push(EntrySplice {
        position,
        removed: 0,
        entries: vec![entry],
    });
}

/// What one removal batch leaves of a column's entries.
struct RemovedEntries {
    retained: Vec<FileEntry>,
    locations: Vec<Location>,
    splices: Vec<EntrySplice>,
    retained_before_selected: usize,
}

impl RemovedEntries {
    fn split(
        entries: Vec<FileEntry>,
        removals: &HashSet<Location>,
        selected_position: Option<usize>,
    ) -> Self {
        let mut split = Self {
            retained: Vec::with_capacity(entries.len()),
            locations: Vec::new(),
            splices: Vec::new(),
            retained_before_selected: 0,
        };
        for (position, entry) in entries.into_iter().enumerate() {
            if removals.contains(&entry.location) {
                split.remove(entry.location);
            } else {
                split.retain(
                    entry,
                    selected_position.is_some_and(|selected| position < selected),
                );
            }
        }
        split
    }

    fn remove(&mut self, location: Location) {
        self.locations.push(location);
        let position = self.retained.len();
        match self
            .splices
            .last_mut()
            .filter(|splice| splice.position == position)
        {
            Some(splice) => splice.removed += 1,
            None => self.splices.push(EntrySplice {
                position,
                removed: 1,
                entries: Vec::new(),
            }),
        }
    }

    fn retain(&mut self, entry: FileEntry, before_selected: bool) {
        if before_selected {
            self.retained_before_selected += 1;
        }
        self.retained.push(entry);
    }
}

/// Drops the removed locations from the fill, and returns whether a removed
/// cursor entry hands the cursor to a neighbour.
fn forget_removed(
    selectionless_removals: &mut HashSet<Location>,
    column: &mut ColumnState,
    removed: &[Location],
    selected: Option<&Location>,
) -> bool {
    let mut replace_selection = false;
    for location in removed {
        let replace = !selectionless_removals.remove(location);
        if selected == Some(location) {
            replace_selection = replace;
        }
        column.selected_locations.remove(location);
    }
    replace_selection
}

impl ColumnState {
    fn neighbour_after_removal(&mut self, position: usize, preserve_fill: bool) -> Option<usize> {
        let replacement = self.visible_neighbor(position);
        if !preserve_fill && let Some(position) = replacement {
            self.selected_locations
                .insert(self.entries[position].location.clone());
        }
        replacement
    }

    fn neighbour_location(&self, removed_position: Option<usize>) -> Option<Location> {
        removed_position
            .and_then(|position| self.visible_neighbor(position))
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.location.clone())
    }

    /// Moves the fill and load targets from `from` onto `follow`, or drops them
    /// when the moved entry leaves the view.
    fn follow_moved_selection(&mut self, from: &Location, follow: Option<&Location>) {
        match follow {
            Some(to) => self.selected_locations.relocate(from, to.clone()),
            None => {
                self.selected_locations.remove(from);
            }
        }
        for target in [
            &mut self.selection_target,
            &mut self.selection_anchor,
            &mut self.load_cursor,
        ] {
            if target.as_ref() == Some(from) {
                *target = follow.cloned();
            }
        }
    }
}

/// One monitor change applied to a column, tracking where its cursor goes.
struct MonitoredChange<'a> {
    column: &'a mut ColumnState,
    preferences: ViewPreferences,
    preserve_fill: bool,
    selected: Option<Location>,
    splices: Vec<EntrySplice>,
}

impl<'a> MonitoredChange<'a> {
    fn new(column: &'a mut ColumnState, preserve_fill: bool) -> Self {
        Self {
            preferences: column.preferences,
            selected: column.cursor_location(),
            column,
            preserve_fill,
            splices: Vec::new(),
        }
    }

    /// Returns false when the column already lists the entry unchanged.
    fn upsert(&mut self, entry: FileEntry) -> bool {
        let existing = self
            .column
            .entries
            .iter()
            .find(|current| current.location == entry.location);
        if existing == Some(&entry) {
            return false;
        }
        if existing.is_none() {
            self.column
                .selected_locations
                .excludes_new(entry.location.clone());
        }
        upsert_monitored_entry(
            &mut self.column.entries,
            entry,
            self.preferences,
            &mut self.splices,
        );
        true
    }

    fn remove(&mut self, location: &Location, replace_selection: bool) {
        let removed_position = self.column.position_of(location);
        let selected_was_removed = self.selected.as_ref() == Some(location);
        self.column.selected_locations.remove(location);
        remove_monitored_entry(&mut self.column.entries, location, &mut self.splices);
        if selected_was_removed && replace_selection {
            self.selected = self.column.neighbour_location(removed_position);
            self.fill_replacement();
        }
    }

    fn relocate(&mut self, from: &Location, mut entry: FileEntry) {
        if let Some(previous) = self
            .column
            .entries
            .iter()
            .find(|previous| previous.location == *from)
            && same_file_moved(previous, &entry)
        {
            inherit_loaded_details(&mut entry, previous);
        }
        // A hidden destination leaves the filtered view, so the cursor moves as on removal.
        let hides = entry.is_hidden && !self.preferences.show_hidden;
        let removed_position = self.column.position_of(from);
        let follow = (!hides).then(|| entry.location.clone());
        self.column.follow_moved_selection(from, follow.as_ref());
        remove_monitored_entry(&mut self.column.entries, from, &mut self.splices);
        if self.selected.as_ref() == Some(from) {
            self.selected = follow.or_else(|| self.column.neighbour_location(removed_position));
            if hides {
                self.fill_replacement();
            }
        }
        upsert_monitored_entry(
            &mut self.column.entries,
            entry,
            self.preferences,
            &mut self.splices,
        );
    }

    fn fill_replacement(&mut self) {
        if !self.preserve_fill
            && let Some(replacement) = &self.selected
        {
            self.column.selected_locations.insert(replacement.clone());
        }
    }

    fn finish(self) -> EntrySpliceApplication {
        if self.splices.is_empty() {
            return None;
        }
        let column = self.column;
        column.selected = self
            .selected
            .and_then(|location| column.position_of(&location));
        column.invalidate_spliced_entry_indexes();
        column.retain_selected_locations(false);
        column.settle_listed_state();
        Some((self.splices, column.selected))
    }
}

/// A move that kept the file's kind, size, modification time and extension
/// still describes the same content.
fn same_file_moved(previous: &FileEntry, entry: &FileEntry) -> bool {
    previous.kind == entry.kind
        && matches!(entry.size, MetadataValue::Known(_))
        && previous.size == entry.size
        && matches!(entry.modified_unix_seconds, MetadataValue::Known(_))
        && previous.modified_unix_seconds == entry.modified_unix_seconds
        && std::path::Path::new(&previous.native_name).extension()
            == std::path::Path::new(&entry.native_name).extension()
}

/// Monitor entries contain stat data, not the details already loaded for this file.
fn inherit_loaded_details(entry: &mut FileEntry, previous: &FileEntry) {
    if entry.image_dimensions == MetadataValue::Unknown {
        entry.image_dimensions = previous.image_dimensions.clone();
    }
    if entry.child_count == MetadataValue::Unknown {
        entry.child_count = previous.child_count.clone();
    }
    if entry.duration_seconds == MetadataValue::Unknown {
        entry.duration_seconds = previous.duration_seconds.clone();
    }
}
