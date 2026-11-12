// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{model::Location, services::Responsiveness};

use super::listing::DirectoryLoad;
use super::{Browser, BrowserEvent, KeptListing, MAX_INCREMENTAL_OPERATION_UPDATES, MonitorState};

impl Browser {
    pub fn refresh_columns_at(self: &Rc<Self>, location: &Location) {
        self.refresh_columns_at_many([location]);
    }

    pub(super) fn refresh_columns_at_many<'a>(
        self: &Rc<Self>,
        locations: impl IntoIterator<Item = &'a Location>,
    ) {
        for depth in self.depths_showing(locations) {
            self.refresh_column(depth);
        }
    }

    pub(super) fn depths_showing<'a>(
        &self,
        locations: impl IntoIterator<Item = &'a Location>,
    ) -> Vec<usize> {
        let locations: Vec<_> = locations.into_iter().collect();
        let state = self.state.borrow();
        let mut depths = Vec::new();
        let mut depth = 0;
        while let Some(open_location) = state.location_at(depth) {
            if locations.iter().any(|location| {
                &open_location == *location
                    || (open_location.is_recent_root() && location.is_recent_root())
                    || open_location.contains_camera_photo_location(location)
            }) {
                depths.push(depth);
            }
            depth += 1;
        }
        depths
    }

    pub fn refresh_after_cancellation(self: &Rc<Self>, roots: &HashSet<Location>) {
        self.refresh_columns_at_or_below(roots);
    }

    pub fn column_has_monitor(&self, depth: usize) -> bool {
        self.monitors
            .borrow()
            .get(depth)
            .is_some_and(|monitor| monitor.state.get() == MonitorState::Active)
    }

    fn open_locations(&self) -> Vec<(usize, Location)> {
        let state = self.state.borrow();
        let mut locations = Vec::new();
        let mut depth = 0;
        while let Some(location) = state.location_at(depth) {
            locations.push((depth, location));
            depth += 1;
        }
        locations
    }

    pub(super) fn refresh_columns_at_or_below(self: &Rc<Self>, roots: &HashSet<Location>) {
        for (depth, location) in self.open_locations() {
            let affected = location_or_ancestor_is_affected(&location, roots)
                || roots
                    .iter()
                    .any(|root| location.contains_camera_photo_location(root));
            if affected && !self.column_has_monitor(depth) {
                self.refresh_column(depth);
            }
        }
    }

    /// A Recent or camera view in `rescanning` reloads once the operation
    /// ends, so a bulk removal leaves it to that reload.
    pub(super) fn remove_deleted_locations(
        self: &Rc<Self>,
        locations: &[Location],
        rescanning: &[usize],
    ) {
        let open_locations = self.open_locations();
        let by_folder = group_by_deletion_parent(locations);
        for (depth, watched) in open_locations {
            let aggregate = watched.is_recent_root() || watched.is_camera_photo_root();
            if aggregate && locations.len() > MAX_INCREMENTAL_OPERATION_UPDATES {
                if !rescanning.contains(&depth) {
                    self.refresh_column(depth);
                }
                continue;
            }
            let removed = match aggregate {
                true => aggregated_removals(&watched, locations),
                false => by_folder.get(&watched).cloned().unwrap_or_default(),
            };
            self.remove_listed(depth, &watched, removed);
        }
    }

    fn remove_listed(self: &Rc<Self>, depth: usize, watched: &Location, removed: Vec<Location>) {
        if removed.is_empty() {
            return;
        }
        let focused_was_removed = self
            .focused_item()
            .is_some_and(|(focused_depth, _, entry)| {
                focused_depth == depth && removed.contains(&entry.location)
            });
        let application = self
            .state
            .borrow_mut()
            .apply_removals_batch(depth, watched, removed);
        self.publish_live_change(depth, application, focused_was_removed);
    }

    pub fn retry_column(self: &Rc<Self>, depth: usize) {
        self.refresh_column(depth);
    }

    pub(super) fn refresh_column(self: &Rc<Self>, depth: usize) {
        if self.keeps_listing_on_refresh(depth) {
            self.refresh_keeping_listing(depth, false);
            return;
        }
        self.refresh_column_with_reveal(depth, None);
    }

    /// A listed column whose source is not answering keeps its entries while it reloads.
    fn keeps_listing_on_refresh(&self, depth: usize) -> bool {
        let listed = self.settled(depth) || self.preserving_refreshes.borrow().contains(&depth);
        listed
            && self.location_at(depth).is_some_and(|location| {
                self.source.responsiveness(&location) != Responsiveness::Answering
            })
    }

    pub fn refresh_operation_columns(self: &Rc<Self>, depths: &[usize]) {
        for &depth in depths {
            self.refresh_column_preserving_entries(depth);
        }
    }

    pub(super) fn refresh_column_preserving_entries(self: &Rc<Self>, depth: usize) {
        self.refresh_preserving(depth, None);
    }

    pub(in crate::app) fn refresh_keeping_listing(self: &Rc<Self>, depth: usize, background: bool) {
        self.drain_publish(depth);
        let shown = {
            let state = self.state.borrow();
            state
                .columns
                .get(depth)
                .map(|column| (state.request_id_for_depth(depth), column.truncated))
        };
        let Some((request_id, truncated)) = shown else {
            return;
        };
        let truncated = self
            .kept_listings
            .borrow()
            .get(&depth)
            .filter(|kept| Some(kept.request_id) == request_id)
            .map_or(truncated, |kept| kept.truncated);
        self.refresh_preserving(depth, Some((truncated, background)));
    }

    pub(super) fn refresh_preserving(self: &Rc<Self>, depth: usize, kept: Option<(bool, bool)>) {
        let request_id = self.new_request_id();
        let location = self.state.borrow_mut().refresh_column(depth, request_id);
        let Some(location) = location else {
            return;
        };
        Self::refresh_search_indexes_for(&location, None);
        self.preserving_refreshes.borrow_mut().insert(depth);
        let background = match kept {
            Some((truncated, background)) => {
                self.kept_listings.borrow_mut().insert(
                    depth,
                    KeptListing {
                        request_id,
                        truncated,
                        background,
                    },
                );
                background
            }
            None => {
                self.kept_listings.borrow_mut().remove(&depth);
                false
            }
        };
        self.emit(BrowserEvent::ColumnRefreshing { depth });
        let handle = self.request_directory(DirectoryLoad {
            depth,
            location,
            request_id,
            background,
        });
        if let Some(load) = self.loads.borrow_mut().get_mut(depth) {
            *load = handle;
        }
        self.metadata_loads.borrow_mut().remove(&depth);
    }

    pub(super) fn refresh_column_revealing(self: &Rc<Self>, depth: usize, targets: Vec<Location>) {
        self.refresh_column_with_reveal(depth, Some(targets));
    }

    pub(super) fn refresh_column_with_reveal(
        self: &Rc<Self>,
        depth: usize,
        targets: Option<Vec<Location>>,
    ) {
        // An explicit target outranks any position a view would restore after the reload.
        if targets.is_none() && self.location_at(depth).is_some() {
            self.emit(BrowserEvent::ColumnReloading { depth });
        }
        let request_id = self.new_request_id();
        let location = {
            let mut state = self.state.borrow_mut();
            let location = state.reload_column(depth, request_id);
            if location.is_some()
                && let Some(targets) = targets
            {
                state.select_locations_on_load(depth, targets);
            }
            location
        };
        let Some(location) = location else {
            return;
        };
        Self::refresh_search_indexes_for(&location, None);
        self.emit(BrowserEvent::ColumnReloaded { depth });
        let handle = self.request_directory(DirectoryLoad {
            depth,
            location,
            request_id,
            background: false,
        });
        if let Some(load) = self.loads.borrow_mut().get_mut(depth) {
            *load = handle;
        }
        self.metadata_loads.borrow_mut().remove(&depth);
        self.metadata_pending.borrow_mut().remove(&depth);
        self.remote.borrow_mut().clear_depth(depth);
        self.last_batch_selection.borrow_mut().remove(&depth);
        self.cancel_pending_sort_for(depth);
        self.staging.borrow_mut().remove(&depth);
        self.sorting.borrow_mut().remove(&depth);
        self.cancel_publish(depth);
        self.fill_tokens.borrow_mut().retain(|_, fill| {
            self.state.borrow().request_id_for_depth(fill.depth) == Some(fill.directory_request)
        });
    }

    pub fn reload_active(self: &Rc<Self>) {
        if let Some(depth) = self.active_depth() {
            self.refresh_column(depth);
        }
    }

    pub fn refresh_all(self: &Rc<Self>) {
        let depths: Vec<usize> = {
            let state = self.state.borrow();
            (0..state.columns.len()).collect()
        };
        if depths.is_empty() {
            self.reload_active();
            return;
        }
        for depth in depths {
            self.refresh_column(depth);
        }
    }
}

pub(super) fn location_or_ancestor_is_affected(
    location: &Location,
    roots: &HashSet<Location>,
) -> bool {
    std::iter::successors(Some(location.clone()), Location::parent)
        .any(|folder| roots.contains(&folder))
}

pub(super) fn deletion_parent_location(location: &Location) -> Option<Location> {
    if location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
    {
        Some(Location::uri("trash:///"))
    } else {
        location.parent()
    }
}

fn group_by_deletion_parent(locations: &[Location]) -> HashMap<Location, Vec<Location>> {
    let mut by_folder = HashMap::<_, Vec<_>>::new();
    for location in locations {
        if let Some(parent) = deletion_parent_location(location) {
            by_folder.entry(parent).or_default().push(location.clone());
        }
    }
    by_folder
}

/// Recent lists every removed item; a camera view lists those under it.
fn aggregated_removals(watched: &Location, locations: &[Location]) -> Vec<Location> {
    locations
        .iter()
        .filter(|location| {
            watched.is_recent_root() || watched.contains_camera_photo_location(location)
        })
        .cloned()
        .collect()
}
