// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    app::navigation::{LoadState, NavigationState},
    model::{FileEntry, Location, MetadataValue},
    services::{MetadataUpdate, RequestId, file_activity::EndedOperation},
};

use super::super::{Browser, BrowserEvent, SortFill};

#[cfg(test)]
mod tests;

impl Browser {
    pub(in crate::app::browser) fn invalidate_folder_counts(&self, ended: &EndedOperation) {
        let parents: HashSet<&Path> = ended
            .folders
            .iter()
            .chain(&ended.roots)
            .chain(&ended.trees)
            .filter_map(|folder| folder.parent())
            .collect();
        let depths = {
            let state = self.state.borrow();
            let mut loads = self.folder_counts_after_load.borrow_mut();
            state
                .columns
                .iter()
                .enumerate()
                .filter_map(|(depth, column)| {
                    let path = column.location.native_path()?;
                    if !parents.contains(path)
                        && !path.ancestors().any(|parent| ended.trees.contains(parent))
                    {
                        return None;
                    }
                    if column.load_state == LoadState::Loading
                        && let Some(request) = state.request_id_for_depth(depth)
                    {
                        loads.insert(depth, request);
                    }
                    Some(depth)
                })
                .collect::<HashSet<_>>()
        };
        self.invalidate_count_entries(&depths, |location| {
            location.native_path().is_some_and(|path| {
                ended.folders.contains(path)
                    || ended.roots.contains(path)
                    || path.ancestors().any(|parent| ended.trees.contains(parent))
            })
        });
    }

    pub(in crate::app::browser) fn invalidate_loaded_folder_counts(&self, event: &BrowserEvent) {
        let BrowserEvent::LoadFinished { depth, .. } = event else {
            return;
        };
        let request = self.folder_counts_after_load.borrow_mut().remove(depth);
        if request.is_some() && request == self.state.borrow().request_id_for_depth(*depth) {
            // Enumeration can have captured cached counts before the operation ended.
            self.invalidate_count_entries(&HashSet::from([*depth]), |location| {
                location.native_path().is_some()
            });
        }
    }

    fn invalidate_count_entries(
        &self,
        depths: &HashSet<usize>,
        affected: impl Fn(&Location) -> bool,
    ) {
        let mut invalidated = HashMap::new();
        {
            let mut state = self.state.borrow_mut();
            for &depth in depths {
                let Some(column) = state.columns.get_mut(depth) else {
                    continue;
                };
                let locations = column
                    .entries
                    .iter_mut()
                    .filter(|entry| entry.is_directory() && affected(&entry.location))
                    .map(|entry| {
                        entry.child_count = MetadataValue::Unknown;
                        entry.location.clone()
                    })
                    .collect::<HashSet<_>>();
                if !locations.is_empty() {
                    invalidated.insert(depth, locations);
                }
            }
        }
        if invalidated.is_empty() {
            return;
        }
        let cancelled_depths = self
            .fill_tokens
            .borrow_mut()
            .extract_if(|_, fill| {
                let Some(locations) = invalidated.get(&fill.depth) else {
                    return false;
                };
                fill.tokens
                    .retain(|(_, location)| !locations.contains(location));
                fill.tokens.is_empty()
            })
            .map(|(_, fill)| fill.depth)
            .collect::<HashSet<_>>();
        let cancelled = self
            .metadata_loads
            .borrow_mut()
            .extract_if(|depth, _| cancelled_depths.contains(depth))
            .map(|(_, load)| load)
            .collect::<Vec<_>>();
        drop(cancelled);
        self.metadata_pending.borrow_mut().retain(|depth, pending| {
            if let Some(locations) = invalidated.get(depth) {
                pending.retain(|target| !locations.contains(&target.location));
            }
            !pending.is_empty()
        });
        // Views re-admit their live viewport targets; offscreen folders wait for a bind.
        self.emit(BrowserEvent::FolderCountsInvalidated);
    }

    pub(crate) fn apply_thumbnail_metadata(
        &self,
        depth: usize,
        position: usize,
        update: MetadataUpdate,
    ) {
        let mut state = self.state.borrow_mut();
        let Some(request) = state.request_id_for_depth(depth) else {
            return;
        };
        let Some((depth, positions, _)) =
            state.apply_positioned_metadata(request, vec![(position, update)])
        else {
            return;
        };
        let updates = filled_entries(&state, depth, &positions);
        drop(state);
        if !updates.is_empty() {
            self.emit(BrowserEvent::MetadataFilled { depth, updates });
        }
    }

    pub(super) fn receive_metadata(&self, request_id: RequestId, updates: Vec<MetadataUpdate>) {
        let awaiting = self
            .sort_awaiting_fill
            .borrow()
            .as_ref()
            .copied()
            .filter(|fill| fill.fill_request == request_id);
        if let Some(fill) = awaiting {
            self.apply_sort_metadata(fill, updates);
        } else {
            self.apply_viewport_metadata(request_id, updates);
        }
    }

    fn apply_sort_metadata(&self, fill: SortFill, updates: Vec<MetadataUpdate>) {
        // Full sort fills apply by location, independently of viewport positions.
        let mut state = self.state.borrow_mut();
        if let Some((depth, positions)) = state.apply_metadata(fill.directory_request, updates) {
            let filled = filled_entries(&state, depth, &positions);
            tracing::debug!(
                request_id = fill.fill_request.0,
                depth,
                filled = positions.len(),
                "metadata fill applied"
            );
            drop(state);
            self.emit(BrowserEvent::MetadataFilled {
                depth,
                updates: filled,
            });
        }
    }

    fn apply_viewport_metadata(&self, request_id: RequestId, updates: Vec<MetadataUpdate>) {
        let fill = self
            .fill_tokens
            .borrow()
            .get(&request_id)
            .map(|fill| (fill.directory_request, fill.tokens.clone()));
        let Some((directory_request, tokens)) = fill else {
            return;
        };
        let positioned = position_updates(&tokens, updates);
        let mut state = self.state.borrow_mut();
        if let Some((depth, positions, stale)) =
            state.apply_positioned_metadata(directory_request, positioned)
        {
            let filled = filled_entries(&state, depth, &positions);
            tracing::debug!(
                request_id = request_id.0,
                depth,
                filled = positions.len(),
                stale = stale.len(),
                "metadata fill applied"
            );
            drop(state);
            if !filled.is_empty() {
                self.emit(BrowserEvent::MetadataFilled {
                    depth,
                    updates: filled,
                });
            }
        }
    }
}

fn position_updates(
    tokens: &[(usize, Location)],
    updates: Vec<MetadataUpdate>,
) -> Vec<(usize, MetadataUpdate)> {
    let positions: HashMap<&Location, usize> = tokens
        .iter()
        .map(|(position, location)| (location, *position))
        .collect();
    let mut positioned = Vec::with_capacity(updates.len());
    for update in updates {
        if let Some(position) = positions.get(&update.location) {
            positioned.push((*position, update));
        }
    }
    positioned
}

fn filled_entries(
    state: &NavigationState,
    depth: usize,
    positions: &[usize],
) -> Vec<(usize, FileEntry)> {
    positions
        .iter()
        .filter_map(|position| {
            let entry = state.columns.get(depth)?.entries.get(*position)?.clone();
            Some((*position, entry))
        })
        .collect()
}
