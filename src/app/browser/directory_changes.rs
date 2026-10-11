// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

use crate::{
    app::navigation::{EntrySpliceApplication, NavigationPath},
    model::Location,
    services::{DirectoryChange, LoadHandle, LocationValidationError},
};

use super::{Browser, BrowserEvent, StagingLoad};

impl StagingLoad {
    fn record_change(&mut self, watched: &Location, change: DirectoryChange) {
        match &change {
            DirectoryChange::Remove(location) => {
                self.removed.insert(location.clone());
            }
            DirectoryChange::Upsert(entry) => {
                self.removed.remove(&entry.location);
            }
            DirectoryChange::Move { from, entry } => {
                self.removed.insert(from.clone());
                self.removed.remove(&entry.location);
            }
            DirectoryChange::Rescan => {}
        }
        self.deltas.push((watched.clone(), change));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DepartureEvidence {
    RemovalReport,
    /// Reloading it failed, which also covers a removal report that a monitor burst or a
    /// reload superseded. The read error stays unless the folder is confirmed gone.
    FailedLoad,
}

/// A chooser refuses folders outside its boundary, which includes missing ones.
fn departure_confirmed(error: &LocationValidationError) -> bool {
    matches!(
        error,
        LocationValidationError::Missing
            | LocationValidationError::NotDirectory
            | LocationValidationError::Refused(_)
    )
}

pub(super) fn removed_location(change: &DirectoryChange) -> Option<&Location> {
    match change {
        DirectoryChange::Remove(location) | DirectoryChange::Move { from: location, .. } => {
            Some(location)
        }
        DirectoryChange::Upsert(_) | DirectoryChange::Rescan => None,
    }
}

impl Browser {
    pub(super) fn handle_directory_change(
        self: &Rc<Self>,
        depth: usize,
        watched: &Location,
        change: DirectoryChange,
    ) {
        if self.location_at(depth).as_ref() != Some(watched) {
            return;
        }
        if removed_location(&change) == Some(watched) {
            if !self.parent_column_watches(depth, watched) {
                self.handle_root_departure(depth, watched, change);
            }
            return;
        }
        // A rescan reloads the column, which refreshes the indexes itself.
        if !matches!(&change, DirectoryChange::Rescan) {
            Self::refresh_search_indexes_for(watched, Some(&change));
        }
        let removed = (!watched.is_recent_root())
            .then(|| removed_location(&change).cloned())
            .flatten();
        if self.file_operation_defers_changes() {
            self.deferred_file_operation_changes
                .borrow_mut()
                .entry(depth)
                .or_default()
                .push((watched.clone(), change));
            return;
        }
        if matches!(&change, DirectoryChange::Rescan) {
            self.refresh_column(depth);
            return;
        }
        if let Some(change) = self.queue_loading_change(depth, watched, change) {
            self.apply_live_directory_change(depth, watched, change);
        }
        if let Some(removed) = removed {
            self.retire_recent_target(&removed);
        }
    }

    fn file_operation_defers_changes(&self) -> bool {
        self.deletion_operation.get()
            || self.restoration_operation.get()
            || self.transfer_operation.get().is_some()
    }

    /// Deeper columns are entries of the parent column, whose monitor reports their
    /// removal or rename.
    fn parent_column_watches(&self, depth: usize, watched: &Location) -> bool {
        depth.checked_sub(1).is_some_and(|parent| {
            self.column_has_monitor(parent) && self.location_at(parent) == watched.parent()
        })
    }

    /// Reacts to the open folder at `depth` reporting its own removal, which is all its
    /// monitor says about a deletion, a rename or a move.
    pub(super) fn handle_root_departure(
        self: &Rc<Self>,
        depth: usize,
        watched: &Location,
        change: DirectoryChange,
    ) {
        if self.location_at(depth).as_ref() != Some(watched)
            || removed_location(&change) != Some(watched)
        {
            return;
        }
        if self.file_operation_defers_changes() {
            self.deferred_file_operation_changes
                .borrow_mut()
                .entry(depth)
                .or_default()
                .push((watched.clone(), change));
            return;
        }
        self.check_departed_directory(depth, watched.clone(), DepartureEvidence::RemovalReport);
    }

    /// Only a folder known to have existed can have departed.
    pub(super) fn check_failed_root_load(self: &Rc<Self>, depth: usize) {
        let Some(location) = self.location_at(depth).filter(|_| depth == 0) else {
            return;
        };
        if self
            .root_identity
            .borrow()
            .as_ref()
            .is_some_and(|(known, _)| *known == location)
        {
            self.check_departed_directory(0, location, DepartureEvidence::FailedLoad);
        }
    }

    /// A folder that exists again reloads in place, which also renews its monitor. A
    /// folder renamed within its parent is followed; otherwise the nearest existing
    /// ancestor is restored. Neither adds a history entry.
    pub(super) fn check_departed_directory(
        self: &Rc<Self>,
        depth: usize,
        departed: Location,
        evidence: DepartureEvidence,
    ) {
        let generation = self.navigation_generation();
        let weak = Rc::downgrade(self);
        let probed = departed.clone();
        let answered = Rc::new(Cell::new(false));
        let answer = answered.clone();
        let emit = Rc::new(move |result: Result<(), LocationValidationError>| {
            answer.set(true);
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if !browser.departure_check_is_current(generation, depth, &probed) {
                return;
            }
            match result {
                Ok(()) if evidence == DepartureEvidence::RemovalReport => {
                    browser.restore_columns(depth + 1)
                }
                Ok(()) => {}
                Err(error) if departure_confirmed(&error) => {
                    browser.find_renamed_directory(generation, depth, probed.clone(), evidence)
                }
                // An unreachable folder (network drop, unmount, permissions) may come back:
                // keeping the listing beats leaving or reloading over a transient error.
                Err(_) => {}
            }
        });
        let probe = self.source.validate_location_async(departed, emit);
        self.keep_departure_probe(generation, &answered, probe);
    }

    fn departure_check_is_current(
        &self,
        generation: u64,
        depth: usize,
        departed: &Location,
    ) -> bool {
        self.navigation_generation() == generation
            && self.location_at(depth).as_ref() == Some(departed)
    }

    /// A step that already answered may have started the next one, which must stay.
    fn keep_departure_probe(&self, generation: u64, answered: &Cell<bool>, probe: LoadHandle) {
        if !answered.get() && self.navigation_generation() == generation {
            self.departure_probe.replace(Some(probe));
        }
    }

    fn restore_columns(self: &Rc<Self>, len: usize) {
        let path = (0..len)
            .map_while(|depth| self.location_at(depth))
            .collect();
        self.restore_path(NavigationPath::from_locations(path));
    }

    fn find_renamed_directory(
        self: &Rc<Self>,
        generation: u64,
        depth: usize,
        departed: Location,
        evidence: DepartureEvidence,
    ) {
        let identity = (depth == 0)
            .then(|| self.root_identity.borrow().clone())
            .flatten()
            .filter(|(location, _)| *location == departed)
            .map(|(_, identity)| identity);
        let (Some(identity), Some(parent)) = (identity, departed.parent()) else {
            self.leave_departed_directory(generation, depth, departed, evidence);
            return;
        };
        let weak = Rc::downgrade(self);
        let probed = departed.clone();
        let answered = Rc::new(Cell::new(false));
        let answer = answered.clone();
        let emit = Rc::new(move |found: Option<Location>| {
            answer.set(true);
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if !browser.departure_check_is_current(generation, depth, &probed) {
                return;
            }
            match found.filter(|found| *found != probed && browser.source.allows_navigation(found))
            {
                Some(renamed) => browser.follow_renamed_location(&probed, &renamed),
                None => {
                    browser.leave_departed_directory(generation, depth, probed.clone(), evidence)
                }
            }
        });
        let probe = self.source.find_by_identity(parent, identity, emit);
        self.keep_departure_probe(generation, &answered, probe);
    }

    fn leave_departed_directory(
        self: &Rc<Self>,
        generation: u64,
        depth: usize,
        departed: Location,
        evidence: DepartureEvidence,
    ) {
        if depth > 0 {
            self.restore_columns(depth);
            return;
        }
        let ancestors = std::iter::successors(departed.parent(), Location::parent)
            .filter(|ancestor| self.source.allows_navigation(ancestor))
            .collect();
        self.probe_departure_ancestors(generation, departed, ancestors, evidence);
    }

    fn probe_departure_ancestors(
        self: &Rc<Self>,
        generation: u64,
        departed: Location,
        mut ancestors: VecDeque<Location>,
        evidence: DepartureEvidence,
    ) {
        let Some(ancestor) = ancestors.pop_front() else {
            // Nothing the source allows exists above it: show the folder's read error.
            if evidence == DepartureEvidence::RemovalReport {
                self.refresh_column(0);
            }
            return;
        };
        let weak = Rc::downgrade(self);
        let probed = ancestor.clone();
        let remaining = RefCell::new(Some(ancestors));
        let answered = Rc::new(Cell::new(false));
        let answer = answered.clone();
        let emit = Rc::new(move |result: Result<(), LocationValidationError>| {
            answer.set(true);
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if !browser.departure_check_is_current(generation, 0, &departed) {
                return;
            }
            let Some(ancestors) = remaining.borrow_mut().take() else {
                return;
            };
            match result {
                Ok(()) => {
                    browser.restore_path(NavigationPath::from_locations(vec![probed.clone()]))
                }
                Err(error) if departure_confirmed(&error) => browser.probe_departure_ancestors(
                    generation,
                    departed.clone(),
                    ancestors,
                    evidence,
                ),
                Err(_) => {}
            }
        });
        let probe = self.source.validate_location_async(ancestor, emit);
        self.keep_departure_probe(generation, &answered, probe);
    }

    /// Keeps pane filters, and any search sharing their index, in step with the listing
    /// of `watched`. A move also rebases indexes rooted at or below the moved entry.
    pub(super) fn refresh_search_indexes_for(watched: &Location, change: Option<&DirectoryChange>) {
        if watched.is_recent_root() {
            return;
        }
        let Some(directory) = watched.native_path() else {
            return;
        };
        if let Some(DirectoryChange::Move { from, entry }) = change
            && let (Some(from), Some(to)) = (from.native_path(), entry.location.native_path())
        {
            crate::services::rebase_search_indexes(from, to, crate::services::RenameScope::Moved);
        }
        crate::services::refresh_search_indexes_for_directory(directory);
    }

    pub(super) fn retire_recent_target(self: &Rc<Self>, removed: &Location) {
        let recent = (0..)
            .map_while(|depth| self.location_at(depth).map(|location| (depth, location)))
            .filter(|(_, location)| location.is_recent_root())
            .collect::<Vec<_>>();
        for (depth, watched) in recent {
            let change = DirectoryChange::Remove(removed.clone());
            if let Some(change) = self.queue_loading_change(depth, &watched, change) {
                self.drain_publish(depth);
                let application = self
                    .state
                    .borrow_mut()
                    .apply_directory_change(depth, &watched, change);
                self.publish_external_change(|| {
                    self.publish_live_change(depth, application, false);
                });
            }
        }
    }

    fn queue_loading_change(
        &self,
        depth: usize,
        watched: &Location,
        change: DirectoryChange,
    ) -> Option<DirectoryChange> {
        if let Some(staging) = self.staging.borrow_mut().get_mut(&depth) {
            staging.record_change(watched, change);
            return None;
        }
        if let Some(sorting) = self.sorting.borrow_mut().get_mut(&depth) {
            sorting.deltas.push((watched.clone(), change));
            return None;
        }
        Some(change)
    }

    fn apply_live_directory_change(
        self: &Rc<Self>,
        depth: usize,
        watched: &Location,
        change: DirectoryChange,
    ) {
        // Finish staged tails before emitting splices into their positions.
        self.drain_publish(depth);
        let path_update = self
            .state
            .borrow()
            .path_after_external_change(depth, &change);
        if let Some(path) = path_update
            && !matches!(&change, DirectoryChange::Move { .. })
        {
            self.restore_path(path);
            return;
        }
        let focused_was_removed = matches!(&change, DirectoryChange::Remove(location)
            if self.focused_item().is_some_and(|(focused_depth, _, entry)|
                focused_depth == depth && entry.location == *location));
        let relocation = match &change {
            DirectoryChange::Move { from, entry } => Some((from.clone(), entry.location.clone())),
            _ => None,
        };
        let application = self
            .state
            .borrow_mut()
            .apply_directory_change(depth, watched, change);
        self.publish_external_change(|| {
            self.publish_live_change(depth, application, focused_was_removed);
        });
        if let Some((from, to)) = relocation {
            self.relocate_open_columns(&from, &to);
        }
    }

    /// Marks the `FocusChanged` that `publish` emits as following an outside change.
    pub(super) fn publish_external_change(&self, publish: impl FnOnce()) {
        let was = self.external_change_focus.replace(true);
        publish();
        self.external_change_focus.set(was);
    }

    pub(super) fn publish_live_change(
        &self,
        depth: usize,
        application: EntrySpliceApplication,
        focused_was_removed: bool,
    ) {
        let Some((splices, selected)) = application else {
            return;
        };
        self.emit(BrowserEvent::EntriesSpliced { depth, splices });
        if self.active_depth() == Some(depth) && (selected.is_none() || focused_was_removed) {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: selected,
                triggered_by_removal: focused_was_removed,
            });
        }
    }
}
