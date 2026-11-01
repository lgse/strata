// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use crate::{model::{FileEntry, Location, MetadataValue}, services::{MetadataUpdate, RequestId}};

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
        let mut positions = Vec::with_capacity(updates.len());
        if index.len() == column.entries.len() {
            for (location, update) in updates {
                if let Some(&position) = index.get(location)
                    && apply_metadata_update(&mut column.entries[position], update)
                {
                    positions.push(position);
                }
            }
        } else {
            // Providers may repeat a location; keep updating every matching entry.
            for (position, entry) in column.entries.iter_mut().enumerate() {
                if let Some(update) = updates.get(&entry.location)
                    && apply_metadata_update(entry, update)
                {
                    positions.push(position);
                }
            }
        }
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
    pub fn column_unknown_metadata(&self, depth: usize) -> Option<Vec<(usize, Location)>> {
        let column = self.columns.get(depth)?;
        let gap: Vec<(usize, Location)> = column
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.modified_unix_seconds == MetadataValue::Unknown
                    || (!entry.is_directory() && entry.size == MetadataValue::Unknown)
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

pub(super) fn apply_metadata_update(entry: &mut FileEntry, update: &MetadataUpdate) -> bool {
    let mut changed = false;
    if update.size != MetadataValue::Unknown && entry.size != update.size {
        entry.size = update.size.clone();
        changed = true;
    }
    if update.modified_unix_seconds != MetadataValue::Unknown
        && entry.modified_unix_seconds != update.modified_unix_seconds
    {
        entry.modified_unix_seconds = update.modified_unix_seconds.clone();
        changed = true;
    }
    if update.mode != MetadataValue::Unknown && entry.mode != update.mode {
        entry.mode = update.mode.clone();
        changed = true;
    }
    if update.image_dimensions != MetadataValue::Unknown
        && entry.image_dimensions != update.image_dimensions
    {
        entry.image_dimensions = update.image_dimensions.clone();
        changed = true;
    }
    if update.child_count != MetadataValue::Unknown && entry.child_count != update.child_count {
        entry.child_count = update.child_count.clone();
        changed = true;
    }
    if update.duration_seconds != MetadataValue::Unknown
        && entry.duration_seconds != update.duration_seconds
    {
        entry.duration_seconds = update.duration_seconds.clone();
        changed = true;
    }
    if update.listable != MetadataValue::Unknown && entry.listable != update.listable {
        entry.listable = update.listable.clone();
        changed = true;
    }
    changed
}