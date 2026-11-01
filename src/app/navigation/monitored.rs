// SPDX-License-Identifier: MIT

use std::{cmp::Ordering, collections::HashSet};

use crate::{model::{FileEntry, Location, MetadataValue, SortKey, ViewPreferences}, services::{DirectoryChange, RequestId}};

use super::{EntrySplice, EntrySpliceApplication, LoadState, NavigationState, compare_entries, merge_entries};

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

        let selected_position = column.selected;
        let selected_location = selected_position
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        let mut retained = Vec::with_capacity(column.entries.len());
        let mut removed_locations = Vec::new();
        let mut splices = Vec::<EntrySplice>::new();
        let mut retained_before_selected = 0;
        for (position, entry) in std::mem::take(&mut column.entries).into_iter().enumerate() {
            if removals.contains(&entry.location) {
                removed_locations.push(entry.location);
                let splice_position = retained.len();
                if let Some(splice) = splices
                    .last_mut()
                    .filter(|splice| splice.position == splice_position)
                {
                    splice.removed += 1;
                } else {
                    splices.push(EntrySplice {
                        position: splice_position,
                        removed: 1,
                        entries: Vec::new(),
                    });
                }
            } else {
                if selected_position.is_some_and(|selected| position < selected) {
                    retained_before_selected += 1;
                }
                retained.push(entry);
            }
        }
        column.entries = retained;
        if removed_locations.is_empty() {
            return None;
        }

        let selected_was_removed = selected_location
            .as_ref()
            .is_some_and(|location| removals.contains(location));
        let mut replace_selection = false;
        for location in &removed_locations {
            let replace = !self.selectionless_removals.remove(location);
            if selected_location.as_ref() == Some(location) {
                replace_selection = replace;
            }
            column.selected_locations.remove(location);
        }
        column.selected = if selected_was_removed {
            if replace_selection {
                let replacement = column.visible_neighbor(retained_before_selected);
                if !self.preserve_fill_on_removal
                    && let Some(position) = replacement
                {
                    column
                        .selected_locations
                        .insert(column.entries[position].location.clone());
                }
                replacement
            } else {
                None
            }
        } else {
            selected_location.and_then(|location| {
                column
                    .entries
                    .iter()
                    .position(|entry| entry.location == location)
            })
        };
        column.invalidate_spliced_entry_indexes();
        column.load_state = if column.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
        Some((splices, column.selected))
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
        let column = self.columns.get_mut(depth).expect("column checked");
        let preferences = column.preferences;
        let mut selected_location = column
            .selected
            .and_then(|position| column.entries.get(position))
            .map(|entry| entry.location.clone());
        let mut splices = Vec::new();

        match change {
            DirectoryChange::Upsert(entry) => {
                let existing = column
                    .entries
                    .iter()
                    .find(|current| current.location == entry.location);
                if existing == Some(&entry) {
                    return None;
                }
                if existing.is_none() {
                    column
                        .selected_locations
                        .excludes_new(entry.location.clone());
                }
                upsert_monitored_entry(&mut column.entries, entry, preferences, &mut splices);
            }
            DirectoryChange::Remove(location) => {
                let removed_position = column
                    .entries
                    .iter()
                    .position(|entry| entry.location == location);
                let selected_was_removed = selected_location.as_ref() == Some(&location);
                column.selected_locations.remove(&location);
                remove_monitored_entry(&mut column.entries, &location, &mut splices);
                if selected_was_removed && replace_selection {
                    selected_location = removed_position.and_then(|position| {
                        column
                            .visible_neighbor(position)
                            .and_then(|index| column.entries.get(index))
                            .map(|entry| entry.location.clone())
                    });
                    if !self.preserve_fill_on_removal
                        && let Some(ref replacement) = selected_location
                    {
                        column.selected_locations.insert(replacement.clone());
                    }
                }
            }
            DirectoryChange::Move { from, mut entry } => {
                if let Some(previous) = column
                    .entries
                    .iter()
                    .find(|previous| previous.location == from)
                    && previous.kind == entry.kind
                    && matches!(entry.size, MetadataValue::Known(_))
                    && previous.size == entry.size
                    && matches!(entry.modified_unix_seconds, MetadataValue::Known(_))
                    && previous.modified_unix_seconds == entry.modified_unix_seconds
                    && std::path::Path::new(&previous.native_name).extension()
                        == std::path::Path::new(&entry.native_name).extension()
                {
                    // Monitor entries contain stat data, not the details already loaded for this file.
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
                // A hidden destination leaves the filtered view, so the cursor moves as on removal.
                let hides = entry.is_hidden && !preferences.show_hidden;
                let removed_position = column
                    .entries
                    .iter()
                    .position(|previous| previous.location == from);
                let follow = (!hides).then(|| entry.location.clone());
                if follow.is_some() {
                    column
                        .selected_locations
                        .relocate(&from, entry.location.clone());
                } else {
                    column.selected_locations.remove(&from);
                }
                for target in [
                    &mut column.selection_target,
                    &mut column.selection_anchor,
                    &mut column.load_cursor,
                ] {
                    if target.as_ref() == Some(&from) {
                        *target = follow.clone();
                    }
                }
                remove_monitored_entry(&mut column.entries, &from, &mut splices);
                if selected_location.as_ref() == Some(&from) {
                    selected_location = follow.or_else(|| {
                        removed_position
                            .and_then(|position| column.visible_neighbor(position))
                            .and_then(|index| column.entries.get(index))
                            .map(|neighbor| neighbor.location.clone())
                    });
                    if hides
                        && !self.preserve_fill_on_removal
                        && let Some(ref replacement) = selected_location
                    {
                        column.selected_locations.insert(replacement.clone());
                    }
                }
                upsert_monitored_entry(&mut column.entries, entry, preferences, &mut splices);
            }
            DirectoryChange::Rescan => return None,
        }

        if splices.is_empty() {
            return None;
        }
        column.selected = selected_location.and_then(|location| {
            column
                .entries
                .iter()
                .position(|entry| entry.location == location)
        });
        column.invalidate_spliced_entry_indexes();
        column.retain_selected_locations(false);
        column.load_state = if column.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
        Some((splices, column.selected))
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