// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    model::Location,
    services::{
        CancelledOperation, ItemIdentity, OperationEvent, PlacedSnapshot, ReplacedOriginal,
        TransferReport,
    },
};

use super::{
    Browser, BrowserEvent, OperationCompletion, OperationContext, UndoEntry, push_pending_undo,
};

/// What a finished foreground operation publishes its terminal event with.
pub(super) struct Outcome<'a> {
    pub(super) context: &'a OperationContext,
    pub(super) completion: OperationCompletion,
    pub(super) report: &'a mut TransferReport,
    pub(super) rescanning: &'a [usize],
}

impl Outcome<'_> {
    fn take_report(&mut self) -> TransferReport {
        std::mem::take(self.report)
    }
}

impl Browser {
    pub(super) fn publish_operation_outcome(
        self: &Rc<Self>,
        outcome: Outcome<'_>,
        event: OperationEvent,
    ) {
        match event {
            OperationEvent::Failed { .. } | OperationEvent::TransferFailed { .. } => {
                self.publish_failure(outcome, event)
            }
            OperationEvent::CompletedWithErrors { .. } | OperationEvent::Deleted { .. } => {
                self.publish_deletion_outcome(outcome, event)
            }
            OperationEvent::Restored { .. } | OperationEvent::RestoreCompletedWithErrors { .. } => {
                self.publish_restore_outcome(outcome, event)
            }
            OperationEvent::Cancelled { result, .. } => {
                self.publish_cancelled_operation(outcome, result)
            }
            OperationEvent::Compressed { .. } | OperationEvent::Extracted { .. } => {
                self.publish_archive_outcome(outcome, event)
            }
            OperationEvent::Pasted { .. } => {
                self.publish_completed_transfer(outcome.context, outcome.completion)
            }
            OperationEvent::Renamed { .. }
            | OperationEvent::EntryPlaced { .. }
            | OperationEvent::EntryCreated { .. }
            | OperationEvent::Created { .. } => self.publish_entry_outcome(&outcome, event),
            OperationEvent::TransferProgress { .. }
            | OperationEvent::DeleteProgress { .. }
            | OperationEvent::DeleteEntriesRemoved { .. }
            | OperationEvent::RestoreProgress { .. }
            | OperationEvent::Merged { .. }
            | OperationEvent::ItemsReported { .. }
            | OperationEvent::ItemMoved { .. }
            | OperationEvent::ItemTrashed { .. }
            | OperationEvent::ItemRestored { .. }
            | OperationEvent::ItemKept { .. }
            | OperationEvent::ArchiveStarted { .. }
            | OperationEvent::ArchiveProgress { .. }
            | OperationEvent::FlushingToDevice { .. } => {}
        }
    }

    fn publish_failure(self: &Rc<Self>, mut outcome: Outcome<'_>, event: OperationEvent) {
        let context = outcome.context;
        let (message, password_failure) = match event {
            OperationEvent::Failed { message, .. } if context.rename => {
                self.emit(BrowserEvent::RenameFailed {
                    request_id: Some(context.request_id),
                    message,
                });
                return;
            }
            OperationEvent::Failed {
                message,
                password_failure,
                ..
            } => (message, password_failure),
            OperationEvent::TransferFailed { message, .. } => (message, None),
            _ => return,
        };
        self.refresh_columns_at_or_below(&context.refresh_locations);
        self.emit(BrowserEvent::OperationFailed {
            message,
            password_failure,
            report: outcome.take_report(),
        });
    }

    fn publish_deletion_outcome(self: &Rc<Self>, mut outcome: Outcome<'_>, event: OperationEvent) {
        match event {
            OperationEvent::CompletedWithErrors {
                deleted_locations,
                retryable_locations,
                has_non_retryable_failures,
                message,
                ..
            } => {
                self.remove_deleted_locations(&deleted_locations, outcome.rescanning);
                self.refresh_unmonitored_restore_locations(outcome.context, &outcome.completion);
                self.refresh_retryable_parents(&retryable_locations);
                self.emit(BrowserEvent::OperationCompletedWithErrors {
                    message,
                    retryable_locations,
                    has_non_retryable_failures,
                    report: outcome.take_report(),
                });
            }
            OperationEvent::Deleted { locations, .. } => {
                self.remove_deleted_locations(&locations, outcome.rescanning);
            }
            _ => {}
        }
    }

    /// The folder's Trash capability may have just been found missing.
    fn refresh_retryable_parents(self: &Rc<Self>, retryable_locations: &[Location]) {
        let parents: HashSet<_> = retryable_locations
            .iter()
            .filter_map(Location::parent)
            .filter(|parent| parent.native_path().is_some())
            .collect();
        self.refresh_columns_at_many(&parents);
    }

    fn publish_restore_outcome(self: &Rc<Self>, mut outcome: Outcome<'_>, event: OperationEvent) {
        let (locations, restored, failure) = match event {
            OperationEvent::Restored {
                locations,
                restored,
                ..
            } => (locations, restored, None),
            OperationEvent::RestoreCompletedWithErrors {
                restored_locations,
                restored,
                message,
                ..
            } => (restored_locations, restored, Some(message)),
            _ => return,
        };
        outcome.completion.record_restore_undo(&restored);
        self.remove_deleted_locations(&locations, outcome.rescanning);
        self.refresh_unmonitored_restore_locations(outcome.context, &outcome.completion);
        if outcome.completion.recovering {
            let recovered: &[Location] = match failure {
                None => &restored,
                Some(_) => &[],
            };
            self.publish_recovery(&outcome.context.refresh_locations, recovered);
        }
        if let Some(message) = failure {
            self.emit(BrowserEvent::OperationCompletedWithErrors {
                message,
                retryable_locations: Vec::new(),
                has_non_retryable_failures: true,
                report: outcome.take_report(),
            });
        }
    }

    fn publish_archive_outcome(self: &Rc<Self>, outcome: Outcome<'_>, event: OperationEvent) {
        let undoing = outcome.completion.undoing;
        let select_name = match event {
            OperationEvent::Compressed {
                archive_name,
                archive,
                original,
                placed,
                ..
            } => {
                self.refresh_unmonitored_operation_locations(outcome.context, None);
                if !undoing {
                    record_compressed_undo(archive, original, placed);
                }
                archive_name
            }
            OperationEvent::Extracted {
                first_name,
                created,
                ..
            } => {
                if !undoing {
                    push_pending_undo(UndoEntry::Copy(created));
                }
                first_name.unwrap_or_default()
            }
            _ => return,
        };
        self.emit(BrowserEvent::ArchiveCompleted { select_name });
    }

    fn publish_entry_outcome(self: &Rc<Self>, outcome: &Outcome<'_>, event: OperationEvent) {
        let context = outcome.context;
        match event {
            OperationEvent::Renamed { .. } | OperationEvent::EntryPlaced { .. } => {
                self.emit(BrowserEvent::RenameCompleted {
                    request_id: context.request_id,
                });
            }
            OperationEvent::EntryCreated { location, .. } => {
                if !outcome.completion.undoing {
                    push_pending_undo(UndoEntry::Copy(vec![location.clone()]));
                }
                if self.validation_generation.get() == context.navigation_generation {
                    self.emit(BrowserEvent::EntryCreated { location });
                }
            }
            _ => {}
        }
        self.refresh_unmonitored_operation_locations(context, None);
    }

    /// An unmonitored folder shows what a restore put back, or failed to,
    /// only after a reload.
    fn refresh_unmonitored_restore_locations(
        self: &Rc<Self>,
        context: &OperationContext,
        completion: &OperationCompletion,
    ) {
        if !completion.restoring {
            return;
        }
        let depths: Vec<_> = {
            let restored = context.location_changes.restored.borrow();
            let parents: HashSet<_> = restored
                .iter()
                .filter_map(Location::parent)
                .chain(context.refresh_locations.iter().cloned())
                .collect();
            let state = self.state.borrow();
            (0..)
                .map_while(|depth| Some((depth, state.location_at(depth)?)))
                .filter(|(depth, location)| {
                    !self.column_has_monitor(*depth)
                        && shows_restored(location, &parents, &restored)
                })
                .map(|(depth, _)| depth)
                .collect()
        };
        for depth in depths {
            self.refresh_column(depth);
        }
    }

    /// The view reloads a `revealed` destination itself.
    pub(super) fn refresh_unmonitored_operation_locations(
        self: &Rc<Self>,
        context: &OperationContext,
        revealed: Option<&Location>,
    ) {
        for depth in self.depths_showing(&context.refresh_locations) {
            if !self.column_has_monitor(depth) && self.location_at(depth).as_ref() != revealed {
                self.refresh_column(depth);
            }
        }
    }

    fn publish_completed_transfer(
        self: &Rc<Self>,
        context: &OperationContext,
        completion: OperationCompletion,
    ) {
        let revealed = completion
            .destination
            .clone()
            .filter(|_| self.can_reveal_completed_transfer(context, &completion));
        self.refresh_unmonitored_operation_locations(context, revealed.as_ref());
        if let Some(destination) = revealed {
            self.emit(BrowserEvent::TransferReveal {
                destination,
                locations: completion.reveal_locations,
            });
        }
        self.emit(BrowserEvent::TransferCompleted);
    }

    pub(super) fn can_reveal_completed_transfer(
        &self,
        context: &OperationContext,
        completion: &OperationCompletion,
    ) -> bool {
        if !completion.reveals() {
            return false;
        }
        self.validation_generation.get() == context.navigation_generation
            && self.active_location() == context.origin
    }

    fn publish_cancelled_operation(&self, mut outcome: Outcome<'_>, result: CancelledOperation) {
        let context = outcome.context;
        if context.rename {
            self.emit(BrowserEvent::RenameAbandoned {
                request_id: context.request_id,
            });
        }
        let mut affected_locations = context.refresh_locations.clone();
        affected_locations.extend(result.affected_locations);
        outcome.completion.record_restore_undo(&result.completed);
        if outcome.completion.archiving {
            self.emit(BrowserEvent::ArchiveCompleted {
                select_name: String::new(),
            });
        }
        self.emit(BrowserEvent::OperationCancelled {
            completed: result.completed.len(),
            failed: result.failed.len(),
            not_attempted: result.not_attempted.len(),
            affected_locations,
            report: outcome.take_report(),
        });
    }
}

fn shows_restored(
    location: &Location,
    parents: &HashSet<Location>,
    restored: &HashSet<Location>,
) -> bool {
    parents.contains(location)
        || std::iter::successors(Some(location.clone()), Location::parent)
            .any(|folder| restored.contains(&folder))
}

fn record_compressed_undo(
    archive: Location,
    original: Option<ReplacedOriginal>,
    placed: Option<PlacedSnapshot>,
) {
    match original {
        Some(ReplacedOriginal::Trashed(original)) => {
            push_pending_undo(replaced_archive_undo(archive, original, placed))
        }
        Some(ReplacedOriginal::Deleted) => {}
        None => push_pending_undo(UndoEntry::Copy(vec![archive])),
    }
}

pub(super) fn replaced_archive_undo(
    archive: Location,
    original: Option<ItemIdentity>,
    placed: Option<PlacedSnapshot>,
) -> UndoEntry {
    UndoEntry::Merge {
        created: Vec::new(),
        overwritten: vec![archive.clone()],
        placed: placed
            .map(|placed| HashMap::from([(archive.clone(), placed)]))
            .unwrap_or_default(),
        originals: original
            .map(|original| HashMap::from([(archive, original)]))
            .unwrap_or_default(),
    }
}
