// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, OnceCell},
    collections::HashSet,
};

use crate::{
    model::Location,
    services::{DirectoryChange, RequestId},
};

use super::{ColumnState, LoadState, NavigationPath, NavigationState, SelectedLocations};

impl NavigationState {
    pub fn navigate(&mut self, location: Location, request_id: RequestId) {
        self.record_navigation();
        self.restore(NavigationPath::from_locations(vec![location]), [request_id]);
    }

    pub fn descend(
        &mut self,
        parent_depth: usize,
        location: Location,
        request_id: RequestId,
    ) -> bool {
        if parent_depth >= self.columns.len() {
            return false;
        }

        self.record_navigation();
        self.peek = None;
        self.visual = None;
        self.selection_commit = false;
        self.columns.truncate(parent_depth + 1);
        self.push_column(location, request_id);
        self.active_column = self.columns.len().checked_sub(1);
        true
    }

    pub fn can_go_back(&self) -> bool {
        !self.back_history.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward_history.is_empty()
    }

    pub fn can_go_parent(&self) -> bool {
        self.current_path().and_then(|path| path.parent()).is_some()
    }

    /// Active locations of the back history, most recent first.
    pub fn back_locations(&self) -> Vec<Location> {
        self.back_history
            .iter()
            .rev()
            .filter_map(|path| path.locations().last().cloned())
            .collect()
    }

    /// Goes back `steps` entries; the skipped entries stay reachable with forward.
    pub fn go_back_to(&mut self, steps: usize) -> Option<NavigationPath> {
        if steps == 0 || steps > self.back_history.len() {
            return None;
        }
        if let Some(current) = self.current_path() {
            self.forward_history.push(current);
        }
        for _ in 1..steps {
            let skipped = self.back_history.pop()?;
            self.forward_history.push(skipped);
        }
        self.back_history.pop()
    }

    pub fn go_forward(&mut self) -> Option<NavigationPath> {
        let target = self.forward_history.pop()?;
        if let Some(current) = self.current_path() {
            self.back_history.push(current);
        }
        Some(target)
    }

    pub fn go_parent(&mut self) -> Option<NavigationPath> {
        let target = self.current_path()?.parent()?;
        self.record_navigation();
        Some(target)
    }

    /// The path from `depth` on, recorded as a navigation so Back returns to
    /// the whole strip. None for the first column or past the last.
    pub fn go_to_column(&mut self, depth: usize) -> Option<NavigationPath> {
        if depth == 0 {
            return None;
        }
        let locations = self.current_path()?.locations.get(depth..)?.to_vec();
        if locations.is_empty() {
            return None;
        }
        self.record_navigation();
        Some(NavigationPath::from_locations(locations))
    }

    pub fn restore(
        &mut self,
        path: NavigationPath,
        request_ids: impl IntoIterator<Item = RequestId>,
    ) {
        self.peek = None;
        self.visual = None;
        self.selection_commit = false;
        let previous = std::mem::take(&mut self.columns);
        let kept = |location: &Location| {
            previous
                .iter()
                .find(|column| column.location == *location)
                .map(|column| column.preferences)
        };
        self.columns = path
            .locations
            .into_iter()
            .zip(request_ids)
            .map(|(location, request_id)| ColumnState {
                preferences: self.initial_preferences(&location, kept(&location)),
                location,
                entries: Vec::new(),
                entry_counts: Cell::new(None),
                leftovers: OnceCell::new(),
                surfaced_leftovers: Cell::new(0),
                each_location_once: OnceCell::new(),
                metadata_positions: None,
                selected: None,
                selected_locations: SelectedLocations::default(),
                selection_anchor: None,
                selection_target: None,
                pending_selection: HashSet::new(),
                pending_reveal: Vec::new(),
                selection_from_reveal: false,
                load_state: LoadState::Loading,
                truncated: false,
                can_trash: None,
                can_delete: None,
                request_id,
                select_first_on_load: false,
                preferred_on_load: None,
                open_child_on_load: None,
                reload_cursor: None,
                load_cursor: None,
                type_grouping: self.type_grouping,
            })
            .collect();
        self.active_column = self.columns.len().checked_sub(1);
    }

    pub fn current_path(&self) -> Option<NavigationPath> {
        (!self.columns.is_empty()).then(|| {
            NavigationPath::from_locations(
                self.columns
                    .iter()
                    .map(|column| column.location.clone())
                    .collect(),
            )
        })
    }

    pub fn path_after_external_change(
        &self,
        origin_depth: usize,
        change: &DirectoryChange,
    ) -> Option<NavigationPath> {
        let mut locations = self.current_path()?.locations;
        match change {
            DirectoryChange::Move { from, entry } => {
                let mut changed = false;
                for location in locations.iter_mut().skip(origin_depth + 1) {
                    let Some(rebased) = location.rebase(from, &entry.location) else {
                        continue;
                    };
                    *location = rebased;
                    changed = true;
                }
                changed.then(|| NavigationPath::from_locations(locations))
            }
            DirectoryChange::Remove(removed) => {
                let affected = locations
                    .iter()
                    .enumerate()
                    .skip(origin_depth + 1)
                    .find(|(_, location)| location.is_within(removed))
                    .map(|(depth, _)| depth)?;
                locations.truncate(affected);
                Some(NavigationPath::from_locations(locations))
            }
            DirectoryChange::Upsert(_) | DirectoryChange::Rescan => None,
        }
    }

    pub(super) fn record_navigation(&mut self) {
        if let Some(current) = self.current_path() {
            self.back_history.push(current);
            self.forward_history.clear();
        }
    }

    pub(super) fn push_column(&mut self, location: Location, request_id: RequestId) {
        self.columns.push(ColumnState {
            preferences: self.initial_preferences(&location, None),
            location,
            entries: Vec::new(),
            entry_counts: Cell::new(None),
            leftovers: OnceCell::new(),
            surfaced_leftovers: Cell::new(0),
            each_location_once: OnceCell::new(),
            metadata_positions: None,
            selected: None,
            selected_locations: SelectedLocations::default(),
            selection_anchor: None,
            selection_target: None,
            pending_selection: HashSet::new(),
            pending_reveal: Vec::new(),
            selection_from_reveal: false,
            load_state: LoadState::Loading,
            truncated: false,
            can_trash: None,
            can_delete: None,
            request_id,
            select_first_on_load: false,
            preferred_on_load: None,
            open_child_on_load: None,
            reload_cursor: None,
            load_cursor: None,
            type_grouping: self.type_grouping,
        });
    }
}
