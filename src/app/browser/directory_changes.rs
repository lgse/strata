// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{app::navigation::EntrySplice, model::Location, services::DirectoryChange};

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

fn removed_location(change: &DirectoryChange) -> Option<&Location> {
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
        let removed = (!watched.is_recent_root())
            .then(|| removed_location(&change).cloned())
            .flatten();
        if self.deletion_operation.get()
            || self.restoration_operation.get()
            || self.rename_batch_operation.get()
        {
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
                self.publish_live_change(depth, application, false);
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
        self.publish_live_change(depth, application, focused_was_removed);
        if let Some((from, to)) = relocation {
            self.relocate_open_columns(&from, &to);
        }
    }

    fn publish_live_change(
        &self,
        depth: usize,
        application: Option<(Vec<EntrySplice>, Option<usize>)>,
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
            });
        }
    }
}
