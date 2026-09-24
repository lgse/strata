// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    app::navigation::{EntrySplice, NavigationPath, NavigationState},
    model::{FileEntry, Location},
    services::{DirectoryChange, RenameBatchRecord},
};

use super::{Browser, BrowserEvent, DeferredDirectoryChanges};

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

    fn flush_operation_batch(self: &Rc<Self>, batch: OperationBatch) {
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
                if let Some(publication) = application {
                    self.publish_operation_batch(depth, publication);
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
                positions: publication.positions,
                focused,
                take_focus: false,
            });
        }
        if self.active_depth() == Some(depth) {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: publication.focused,
            });
        }
    }

    /// Publishes completed batch renames with one update per affected depth.
    ///
    /// Monitor traffic during the batch stays deferred, so by the time this
    /// runs most records resolve to entries the deferred flush already
    /// published; those replays are no-ops. Records the monitor missed (or
    /// that arrived without one) apply here, and depths with entries that
    /// cannot be resolved at either end reload to cover the gap.
    pub(super) fn publish_rename_batch(self: &Rc<Self>, renamed: &[RenameBatchRecord]) {
        let mut by_depth: HashMap<usize, Vec<(Location, DirectoryChange)>> = HashMap::new();
        let mut refresh_depths: HashSet<usize> = HashSet::new();
        for record in renamed {
            let Some(parent) = record.original.parent() else {
                continue;
            };
            let depths: Vec<usize> = (0..)
                .map_while(|depth| self.location_at(depth).map(|location| (depth, location)))
                .filter_map(|(depth, location)| (location == parent).then_some(depth))
                .collect();
            let Some(entry) = self.rename_batch_entry(record) else {
                refresh_depths.extend(depths);
                continue;
            };
            for depth in depths {
                by_depth.entry(depth).or_default().push((
                    parent.clone(),
                    DirectoryChange::Move {
                        from: record.original.clone(),
                        entry: entry.clone(),
                    },
                ));
            }
        }
        let mut batches: Vec<_> = by_depth.into_iter().collect();
        batches.sort_by_key(|(depth, _)| *depth);
        for (depth, changes) in batches {
            self.flush_operation_batch(OperationBatch { depth, changes });
        }
        let mut refresh: Vec<_> = refresh_depths.into_iter().collect();
        refresh.sort_unstable();
        for depth in refresh {
            self.refresh_column(depth);
        }
        for record in renamed {
            self.relocate_open_columns(&record.original, &record.current);
        }
    }

    /// Resolves the post-rename entry for a record: the fresh entry when the
    /// monitor already published it, otherwise the stale entry rebased to the
    /// new location so unmonitored columns still update.
    fn rename_batch_entry(&self, record: &RenameBatchRecord) -> Option<FileEntry> {
        if let Some(entry) = self.find_entry_by_location(&record.current) {
            return Some(entry);
        }
        let mut entry = self.find_entry_by_location(&record.original)?;
        let new_name = record.current.file_name()?;
        entry.location = record.current.clone();
        entry.native_name = new_name.clone();
        entry.display_name = new_name.to_string_lossy().into_owned();
        entry.is_hidden = entry.display_name.starts_with('.');
        Some(entry)
    }

    fn find_entry_by_location(&self, location: &Location) -> Option<FileEntry> {
        (0..)
            .map_while(|depth| {
                self.column_snapshot(depth)
                    .map(|snapshot| (depth, snapshot.count))
            })
            .find_map(|(depth, count)| {
                (0..count).find_map(|position| {
                    self.entry_at(depth, position)
                        .filter(|entry| entry.location == *location)
                })
            })
    }
}

/// Summarizes a partially failed rename batch for the error dialog.
pub(super) fn rename_batch_error_summary(total: usize, errors: &[String]) -> String {
    let succeeded = total.saturating_sub(errors.len());
    let mut message = format!("Renamed {succeeded} of {total} items.");
    for error in errors.iter().take(5) {
        message.push_str(&format!("\n• {error}"));
    }
    if errors.len() > 5 {
        message.push_str(&format!("\n…and {} more.", errors.len() - 5));
    }
    message
}
