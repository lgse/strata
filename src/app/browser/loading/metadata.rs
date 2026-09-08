// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use crate::{
    app::navigation::NavigationState,
    model::{FileEntry, Location},
    services::{MetadataUpdate, RequestId},
};

use super::super::{Browser, BrowserEvent, SortFill};

#[cfg(test)]
mod tests;

impl Browser {
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
