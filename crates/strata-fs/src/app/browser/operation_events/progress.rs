// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{model::Location, services::OperationEvent};

use super::{Browser, BrowserEvent, UndoEntry, mark_replay_item_completed};

fn flushes_visible_changes(event: &OperationEvent) -> bool {
    matches!(
        event,
        OperationEvent::DeleteProgress { .. }
            | OperationEvent::TransferProgress { .. }
            | OperationEvent::RestoreProgress { .. }
    )
}

impl Browser {
    /// Returns whether `event` was a progress or per-item event rather than
    /// the operation's terminal.
    pub(super) fn publish_operation_progress(self: &Rc<Self>, event: &OperationEvent) -> bool {
        if self.record_operation_item(event) {
            return true;
        }
        let Some(progress) = self.operation_progress(event) else {
            return false;
        };
        if flushes_visible_changes(event) {
            self.flush_visible_operation_changes();
        }
        self.emit(progress);
        true
    }

    /// Per-item events update the undo and report bookkeeping without
    /// publishing progress.
    fn record_operation_item(&self, event: &OperationEvent) -> bool {
        match event {
            OperationEvent::Merged {
                source,
                created,
                overwritten,
                placed,
                ..
            } => {
                self.merged_undo.borrow_mut().merged(
                    source.clone(),
                    created.clone(),
                    overwritten.clone(),
                    placed.clone(),
                );
            }
            OperationEvent::ItemsReported { report, .. } => {
                self.merged_undo.borrow_mut().rename(&report.renamed);
                self.transfer_report.borrow_mut().extend(report.clone());
            }
            OperationEvent::ItemMoved {
                from, to, merged, ..
            } => {
                if !merged {
                    self.moved_items
                        .borrow_mut()
                        .insert(from.clone(), to.clone());
                }
            }
            OperationEvent::ItemTrashed { .. } | OperationEvent::ItemRestored { .. } => {}
            OperationEvent::ItemKept { location, .. } => {
                self.mark_merged_undo_item_completed(location);
            }
            _ => return false,
        }
        true
    }

    fn operation_progress(&self, event: &OperationEvent) -> Option<BrowserEvent> {
        let progress = match event {
            OperationEvent::DeleteProgress {
                completed,
                total,
                deleted_locations,
                ..
            } => {
                for location in deleted_locations {
                    self.mark_merged_undo_item_completed(location);
                }
                BrowserEvent::DeletionProgress {
                    completed: *completed,
                    total: *total,
                }
            }
            OperationEvent::TransferProgress { .. } => return self.transfer_progress(event),
            OperationEvent::FlushingToDevice { .. } => BrowserEvent::FlushingToDevice,
            OperationEvent::RestoreProgress {
                completed,
                total,
                restored_location,
                ..
            } => {
                self.mark_restored_undo_item(*completed, restored_location.as_ref());
                BrowserEvent::RestorationProgress {
                    completed: *completed,
                    total: *total,
                }
            }
            OperationEvent::DeleteEntriesRemoved { removed, .. } => {
                BrowserEvent::DeletionEntriesRemoved { removed: *removed }
            }
            OperationEvent::ArchiveStarted { total, .. } => {
                BrowserEvent::ArchiveStarted { total: *total }
            }
            OperationEvent::ArchiveProgress {
                completed, total, ..
            } => BrowserEvent::ArchiveProgress {
                completed: *completed,
                total: *total,
            },
            _ => return None,
        };
        Some(progress)
    }

    fn transfer_progress(&self, event: &OperationEvent) -> Option<BrowserEvent> {
        let OperationEvent::TransferProgress {
            completed_items,
            completed_files,
            total_files,
            current_file,
            transferred_bytes,
            total_bytes,
            created_location,
            ..
        } = event
        else {
            return None;
        };
        if let Some(location) = created_location {
            self.created_locations.borrow_mut().push(location.clone());
        }
        Some(BrowserEvent::TransferProgress {
            completed_items: *completed_items,
            completed_files: *completed_files,
            total_files: *total_files,
            current_file: current_file.clone(),
            transferred_bytes: *transferred_bytes,
            total_bytes: *total_bytes,
        })
    }

    fn mark_restored_undo_item(&self, completed: usize, restored: Option<&Location>) {
        let Some(restored) = restored else {
            return;
        };
        for (claim, redo) in [(&self.undo_claim, false), (&self.redo_claim, true)] {
            if mark_claimed_restore(claim.borrow().as_ref(), redo, completed, restored) {
                return;
            }
        }
    }

    /// DeleteProgress carries the processed path, which is how a merge undo
    /// reports trashed `created` items, and ItemKept the ones it left.
    fn mark_merged_undo_item_completed(&self, location: &Location) {
        let claim = self.undo_claim.borrow();
        if let Some((generation, UndoEntry::Merge { .. })) = claim.as_ref() {
            mark_replay_item_completed(false, *generation, location);
        }
    }
}

/// Returns whether the claimed replay accounted for the restored item.
fn mark_claimed_restore(
    claim: Option<&(u64, UndoEntry)>,
    redo: bool,
    completed: usize,
    restored: &Location,
) -> bool {
    match claim {
        // A trash undo reports the trash item, so completion maps by
        // request order onto the recorded locations.
        Some((generation, UndoEntry::Trash { locations, .. })) => {
            let Some(location) = completed
                .checked_sub(1)
                .and_then(|index| locations.get(index))
            else {
                return false;
            };
            mark_replay_item_completed(redo, *generation, location);
            true
        }
        // Merge undos and copy redos restore to the real destination
        // the event reports directly.
        Some((generation, UndoEntry::Merge { .. } | UndoEntry::Copy(_))) => {
            mark_replay_item_completed(redo, *generation, restored);
            true
        }
        _ => false,
    }
}
