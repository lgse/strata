// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{
    app::navigation::{EntrySplice, NavigationPath, NavigationState},
    model::Location,
    services::DirectoryChange,
};

use super::{
    Browser, BrowserEvent, DeferredDirectoryChanges, SelectionUpdate,
    directory_changes::{DepartureEvidence, removed_location},
};

type DirectoryChanges = Vec<(Location, DirectoryChange)>;

struct OperationBatch {
    depth: usize,
    changes: DirectoryChanges,
}

enum BatchAction {
    Refresh,
    Replay,
    Restore(NavigationPath),
    Apply,
}

struct BatchPublication {
    splices: Vec<EntrySplice>,
    focused: Option<usize>,
    positions: Vec<usize>,
}

impl OperationBatch {
    fn action(&self, state: &NavigationState) -> BatchAction {
        if self
            .changes
            .iter()
            .any(|(_, change)| matches!(change, DirectoryChange::Rescan))
        {
            return BatchAction::Refresh;
        }
        if self.changes.iter().any(|(_, change)| {
            matches!(change, DirectoryChange::Move { .. })
                && state
                    .path_after_external_change(self.depth, change)
                    .is_some()
        }) {
            return BatchAction::Replay;
        }
        match self
            .changes
            .iter()
            .find_map(|(_, change)| state.path_after_external_change(self.depth, change))
        {
            Some(path) => BatchAction::Restore(path),
            None => BatchAction::Apply,
        }
    }

    fn apply(self, state: &mut NavigationState) -> Option<BatchPublication> {
        let one_watched = self
            .changes
            .first()
            .map(|(watched, _)| watched)
            .filter(|watched| {
                self.changes
                    .iter()
                    .all(|(candidate, _)| candidate == *watched)
            });
        if let Some(watched) = one_watched
            && self
                .changes
                .iter()
                .all(|(_, change)| matches!(change, DirectoryChange::Upsert(_)))
        {
            let entries = self
                .changes
                .iter()
                .filter_map(|(_, change)| match change {
                    DirectoryChange::Upsert(entry) => Some(entry.clone()),
                    _ => None,
                })
                .collect();
            if let Ok(application) = state.apply_new_entries_batch(self.depth, watched, entries) {
                return application.map(|(splices, focused)| BatchPublication {
                    splices,
                    focused,
                    positions: state.selected_positions(self.depth),
                });
            }
        }
        if let Some(watched) = one_watched
            && self
                .changes
                .iter()
                .all(|(_, change)| matches!(change, DirectoryChange::Remove(_)))
        {
            let locations = self.changes.iter().filter_map(|(_, change)| match change {
                DirectoryChange::Remove(location) => Some(location.clone()),
                _ => None,
            });
            return state
                .apply_removals_batch(self.depth, watched, locations)
                .map(|(splices, focused)| BatchPublication {
                    splices,
                    focused,
                    positions: state.selected_positions(self.depth),
                });
        }

        let mut splices = Vec::new();
        let mut focused = None;
        for (watched, change) in self.changes {
            if let Some((mut next, next_selected)) =
                state.apply_directory_change(self.depth, &watched, change)
            {
                splices.append(&mut next);
                focused = next_selected;
            }
        }
        (!splices.is_empty()).then(|| BatchPublication {
            splices,
            focused,
            positions: state.selected_positions(self.depth),
        })
    }
}

impl Browser {
    pub(super) fn flush_visible_operation_changes(self: &Rc<Self>) {
        let queued = self
            .deferred_file_operation_changes
            .borrow()
            .values()
            .map(Vec::len)
            .sum::<usize>();
        if queued < super::OPERATION_PUBLICATION_BATCH {
            return;
        }
        let changes = self.deferred_file_operation_changes.take();
        self.flush_deferred_file_operation_changes(changes, false);
    }

    pub(super) fn flush_deferred_file_operation_changes(
        self: &Rc<Self>,
        changes: DeferredDirectoryChanges,
        prefer_refresh: bool,
    ) -> bool {
        if changes.is_empty() {
            return false;
        }
        if prefer_refresh {
            // Monitor batches may omit some of the operation's source directories.
            self.refresh_all();
            return true;
        }
        let mut batches: Vec<_> = changes.into_iter().collect();
        batches.sort_by_key(|(depth, _)| *depth);
        for (depth, changes) in batches {
            self.flush_operation_batch(OperationBatch { depth, changes });
        }
        false
    }

    fn flush_operation_batch(self: &Rc<Self>, mut batch: OperationBatch) {
        let rescan = batch
            .changes
            .iter()
            .any(|(_, change)| matches!(change, DirectoryChange::Rescan));
        if rescan {
            self.operation_rescan_depths
                .borrow_mut()
                .insert(batch.depth);
            batch
                .changes
                .retain(|(_, change)| !matches!(change, DirectoryChange::Rescan));
        }
        let depth = batch.depth;
        let current = self.location_at(depth);
        let reported = batch.changes.len();
        batch.changes.retain(|(watched, change)| {
            Some(watched) != current.as_ref() || removed_location(change) != Some(watched)
        });
        let departed = current.filter(|_| batch.changes.len() != reported);
        if !batch.changes.is_empty() {
            self.apply_operation_batch(batch);
        }
        // Checked last, so a folder that turns out to stay keeps an up-to-date listing.
        if let Some(departed) = departed {
            self.check_departed_directory(depth, departed, DepartureEvidence::RemovalReport);
        }
    }

    fn apply_operation_batch(self: &Rc<Self>, batch: OperationBatch) {
        // Classify each depth after earlier batches and their observers have run.
        let action = batch.action(&self.state.borrow());
        match action {
            BatchAction::Refresh => self.refresh_column(batch.depth),
            BatchAction::Replay => {
                for (watched, change) in batch.changes {
                    self.handle_directory_change(batch.depth, &watched, change);
                }
            }
            BatchAction::Restore(path) => self.restore_path(path),
            BatchAction::Apply => {
                let depth = batch.depth;
                self.drain_publish(depth);
                let application = batch.apply(&mut self.state.borrow_mut());
                // The monitor reported these while the operation ran.
                if let Some(publication) = application {
                    self.publish_external_change(|| {
                        self.publish_operation_batch(depth, publication);
                    });
                }
            }
        }
    }

    fn publish_operation_batch(&self, depth: usize, publication: BatchPublication) {
        self.emit(BrowserEvent::EntriesSpliced {
            depth,
            splices: publication.splices,
        });
        if let Some(focused) = publication.focused {
            self.emit(BrowserEvent::SelectionSetChanged {
                depth,
                selection: SelectionUpdate::Positions(publication.positions),
                focused,
                take_focus: false,
            });
        }
        if self.active_depth() == Some(depth) {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: publication.focused,
                triggered_by_removal: false,
            });
        }
    }
}
