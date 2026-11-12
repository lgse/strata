// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use crate::{
    model::{FileEntry, Location, MetadataValue},
    services::{MetadataUpdate, RequestId},
};

use super::{ColumnState, NavigationState};

impl NavigationState {
    /// Order never changes, so views can refresh rows in place.
    pub fn apply_metadata(
        &mut self,
        request_id: RequestId,
        updates: Vec<MetadataUpdate>,
    ) -> Option<(usize, Vec<usize>)> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        let updates: HashMap<&Location, &MetadataUpdate> = updates
            .iter()
            .map(|update| (&update.location, update))
            .collect();
        if updates.is_empty() {
            return None;
        }
        column.index_metadata_positions();
        let index = column.metadata_positions.as_ref().expect("indexed entries");
        let mut positions = if index.len() == column.entries.len() {
            update_indexed_entries(&mut column.entries, index, updates)
        } else {
            // Providers may repeat a location; keep updating every matching entry.
            update_every_matching_entry(&mut column.entries, &updates)
        };
        if positions.is_empty() {
            return None;
        }
        positions.sort_unstable();
        Some((depth, positions))
    }

    pub(in crate::app) fn clear_metadata_positions(&mut self, depth: usize) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.metadata_positions = None;
        }
    }

    /// Stale rows keep their placeholders and retry on the next bind.
    pub fn apply_positioned_metadata(
        &mut self,
        request_id: RequestId,
        updates: Vec<(usize, MetadataUpdate)>,
    ) -> Option<(usize, Vec<usize>, Vec<Location>)> {
        let (depth, column) = self.column_for_request_mut(request_id)?;
        let mut positions = Vec::new();
        let mut stale = Vec::new();
        for (position, update) in &updates {
            let current = column.entries.get(*position);
            if current.is_some_and(|entry| entry.location == update.location) {
                let entry = column.entries.get_mut(*position).expect("position checked");
                if apply_metadata_update(entry, update) {
                    positions.push(*position);
                }
            } else {
                stale.push(update.location.clone());
            }
        }
        if positions.is_empty() && stale.is_empty() {
            return None;
        }
        Some((depth, positions, stale))
    }

    pub fn resolve_entry_at(
        &mut self,
        depth: usize,
        position: usize,
        location: &Location,
    ) -> Option<(usize, FileEntry)> {
        let column = self.columns.get_mut(depth)?;
        let position = if column
            .entries
            .get(position)
            .is_some_and(|entry| &entry.location == location)
        {
            position
        } else {
            column.index_metadata_positions();
            *column.metadata_positions.as_ref()?.get(location)?
        };
        Some((position, column.entries.get(position)?.clone()))
    }

    /// Directories qualify for mtime only; directory size stays unknown by design.
    pub fn column_unknown_sort_metadata(
        &self,
        depth: usize,
        key: crate::model::SortKey,
    ) -> Option<Vec<(usize, Location)>> {
        let column = self.columns.get(depth)?;
        let gap: Vec<(usize, Location)> = column
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if key == crate::model::SortKey::Created {
                    entry.created_unix_seconds == MetadataValue::Unknown
                } else {
                    entry.modified_unix_seconds == MetadataValue::Unknown
                        || (!entry.is_directory() && entry.size == MetadataValue::Unknown)
                }
            })
            .map(|(position, entry)| (position, entry.location.clone()))
            .collect();
        if gap.is_empty() {
            return None;
        }
        Some(gap)
    }
}

impl ColumnState {
    pub(super) fn index_metadata_positions(&mut self) {
        // Sort chunks and shifted viewport targets share one index per structural revision.
        if self.metadata_positions.is_none() {
            self.metadata_positions = Some(
                self.entries
                    .iter()
                    .enumerate()
                    .map(|(position, entry)| (entry.location.clone(), position))
                    .collect(),
            );
        }
    }
}

type MetadataUpdates<'a> = HashMap<&'a Location, &'a MetadataUpdate>;

fn update_indexed_entries(
    entries: &mut [FileEntry],
    index: &HashMap<Location, usize>,
    updates: MetadataUpdates<'_>,
) -> Vec<usize> {
    let mut positions = Vec::with_capacity(updates.len());
    for (location, update) in updates {
        if let Some(&position) = index.get(location)
            && apply_metadata_update(&mut entries[position], update)
        {
            positions.push(position);
        }
    }
    positions
}

fn update_every_matching_entry(
    entries: &mut [FileEntry],
    updates: &MetadataUpdates<'_>,
) -> Vec<usize> {
    let mut positions = Vec::with_capacity(updates.len());
    for (position, entry) in entries.iter_mut().enumerate() {
        if let Some(update) = updates.get(&entry.location)
            && apply_metadata_update(entry, update)
        {
            positions.push(position);
        }
    }
    positions
}

/// Takes a known `update` value, and returns whether it changed `current`.
fn merge_known<T: Clone + PartialEq>(
    current: &mut MetadataValue<T>,
    update: &MetadataValue<T>,
) -> bool {
    if *update == MetadataValue::Unknown || current == update {
        return false;
    }
    *current = update.clone();
    true
}

pub(super) fn apply_metadata_update(entry: &mut FileEntry, update: &MetadataUpdate) -> bool {
    let update_size = if entry.is_computer_item() {
        &MetadataValue::Unknown
    } else {
        &update.size
    };
    let update_child_count = if entry.is_computer_item() {
        &MetadataValue::Unknown
    } else {
        &update.child_count
    };
    let changed = [
        merge_known(&mut entry.size, update_size),
        merge_known(
            &mut entry.modified_unix_seconds,
            &update.modified_unix_seconds,
        ),
        merge_known(
            &mut entry.created_unix_seconds,
            &update.created_unix_seconds,
        ),
        merge_known(&mut entry.mode, &update.mode),
        merge_known(&mut entry.image_dimensions, &update.image_dimensions),
        merge_known(&mut entry.child_count, update_child_count),
        merge_known(&mut entry.duration_seconds, &update.duration_seconds),
        merge_known(&mut entry.listable, &update.listable),
    ];
    changed.contains(&true)
}
