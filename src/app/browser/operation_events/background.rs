// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use crate::services::{LoadHandle, OperationEvent, OperationRequestId};

use super::super::push_background_undo;
use super::{
    Browser, BrowserEvent, MergeUndoState, OperationCompletion, OperationContext, UndoEntry,
};

pub(crate) struct BackgroundOperation {
    pub(crate) load: RefCell<Option<LoadHandle>>,
    pub(crate) cancel_pending: Cell<bool>,
    pub(crate) cancel_requested: Cell<bool>,
    copying: bool,
    deleting: bool,
    deletion_permanent: bool,
    destination: Option<crate::model::Location>,
    reveal: bool,
    created: RefCell<Vec<crate::model::Location>>,
    merged: RefCell<MergeUndoState>,
}

impl Browser {
    pub(crate) fn backgroundable_operation(&self) -> Option<OperationRequestId> {
        self.current_operation.get().filter(|_| {
            self.operation_backgroundable.get()
                && self.undo_claim.borrow().is_none()
                && self.redo_claim.borrow().is_none()
        })
    }

    pub(crate) fn operation_description(&self) -> String {
        self.operation_description.borrow().clone()
    }

    pub(crate) fn operation_destination_description(&self) -> String {
        self.operation_destination_description.borrow().clone()
    }

    pub(crate) fn background_file_operation(
        self: &Rc<Self>,
        request_id: OperationRequestId,
    ) -> bool {
        if self.backgroundable_operation() != Some(request_id) {
            return false;
        }
        let job = Rc::new(BackgroundOperation {
            load: RefCell::new(self.operation_load.take()),
            cancel_pending: Cell::new(self.transfer_cancel_pending.replace(false)),
            cancel_requested: Cell::new(self.operation_cancel_requested.get()),
            copying: self.transfer_operation.take() == Some(false),
            deleting: self.deletion_operation.replace(false),
            deletion_permanent: self.deletion_permanent.replace(false),
            destination: self.transfer_destination.take(),
            reveal: self.transfer_reveal.replace(true),
            created: RefCell::new(self.created_locations.take()),
            merged: RefCell::new(self.merged_undo.take()),
        });
        self.background_operations
            .borrow_mut()
            .insert(request_id, job);
        self.current_operation.set(None);
        self.archive_operation.set(false);
        self.operation_backgroundable.set(false);
        self.operation_description.borrow_mut().clear();
        self.operation_destination_description.borrow_mut().clear();
        let changes = self.deferred_file_operation_changes.take();
        self.flush_deferred_file_operation_changes(changes, false);
        true
    }

    pub(crate) fn has_foreground_operation(&self) -> bool {
        self.current_operation.get().is_some()
    }

    pub(crate) fn has_background_operations(&self) -> bool {
        !self.background_operations.borrow().is_empty()
    }

    pub(crate) fn cancel_background_operations(&self) {
        let ids: Vec<_> = self
            .background_operations
            .borrow()
            .keys()
            .copied()
            .collect();
        for id in ids {
            self.cancel_operation(id);
        }
    }

    pub(crate) fn cancel_operation(&self, request_id: OperationRequestId) {
        let job = self
            .background_operations
            .borrow()
            .get(&request_id)
            .cloned();
        if let Some(job) = job {
            job.cancel_requested.set(true);
            job.cancel_pending.set(job.copying);
            let load = job.load.take();
            drop(load);
        } else if self.is_current_operation(request_id) {
            self.cancel_file_operation();
        }
    }

    pub(super) fn handle_background_operation(
        self: &Rc<Self>,
        context: &OperationContext,
        event: OperationEvent,
        job: Rc<BackgroundOperation>,
    ) {
        let emit = |event| {
            self.emit(BrowserEvent::BackgroundOperation {
                request_id: context.request_id,
                event: Box::new(event),
            })
        };
        match event {
            OperationEvent::TransferProgress {
                completed_items,
                completed_files,
                total_files,
                current_file,
                transferred_bytes,
                total_bytes,
                created_location,
                ..
            } => {
                if let Some(location) = created_location {
                    job.created.borrow_mut().push(location);
                }
                emit(BrowserEvent::TransferProgress {
                    completed_items,
                    completed_files,
                    total_files,
                    current_file,
                    transferred_bytes,
                    total_bytes,
                });
            }
            OperationEvent::DeleteProgress {
                completed, total, ..
            } => {
                emit(BrowserEvent::DeletionProgress { completed, total });
            }
            OperationEvent::Merged {
                source,
                created,
                overwritten,
                ..
            } => {
                let mut merged = job.merged.borrow_mut();
                merged.sources.insert(source);
                merged.created.extend(created);
                merged.overwritten.extend(overwritten);
            }
            OperationEvent::FlushingToDevice { .. } => {
                emit(BrowserEvent::FlushingToDevice);
            }
            OperationEvent::ArchiveStarted { total, .. } => {
                emit(BrowserEvent::ArchiveStarted { total });
            }
            OperationEvent::ArchiveProgress {
                completed, total, ..
            } => {
                emit(BrowserEvent::ArchiveProgress { completed, total });
            }
            event => {
                // Finalize only this request; foreground selection/removal and replay
                // state belongs to a different operation and must remain untouched.
                self.background_operations
                    .borrow_mut()
                    .remove(&context.request_id);
                let completion = OperationCompletion {
                    moving: job.copying.then_some(false),
                    deleting: job.deleting,
                    deletion_permanent: job.deletion_permanent,
                    restoring: false,
                    archiving: !job.copying && !job.deleting,
                    destination: job.destination.clone(),
                    reveal: job.reveal,
                    file_operation_refreshed: false,
                    undoing: false,
                    reveal_locations: job.created.borrow().clone(),
                };
                completion.record_trash_undo(&event, push_background_undo);
                if job.copying {
                    completion.record_transfer_undo(
                        &[],
                        job.created.take(),
                        job.merged.take(),
                        push_background_undo,
                    );
                    emit(BrowserEvent::TransferFinished {
                        moved_locations: Vec::new(),
                    });
                }
                self.refresh_unmonitored_operation_locations(context);
                match event {
                    OperationEvent::Pasted { .. } => {
                        if self.last_started_operation() == Some(context.request_id)
                            && self.can_reveal_completed_transfer(context, &completion)
                            && let Some(destination) = completion.destination
                        {
                            emit(BrowserEvent::TransferReveal {
                                destination,
                                locations: completion.reveal_locations,
                            });
                        }
                        emit(BrowserEvent::TransferCompleted);
                    }
                    OperationEvent::Deleted { locations, .. } => {
                        self.remove_deleted_locations(&locations);
                        emit(BrowserEvent::DeletionFinished { succeeded: true });
                    }
                    OperationEvent::CompletedWithErrors {
                        deleted_locations,
                        retryable_locations,
                        has_non_retryable_failures,
                        message,
                        ..
                    } => {
                        self.remove_deleted_locations(&deleted_locations);
                        emit(BrowserEvent::OperationCompletedWithErrors {
                            retryable_locations,
                            has_non_retryable_failures,
                            message,
                        });
                    }
                    OperationEvent::Compressed {
                        archive_name,
                        archive,
                        original,
                        ..
                    } => {
                        push_background_undo(if let Some(original) = original {
                            UndoEntry::Merge {
                                created: Vec::new(),
                                overwritten: vec![archive.clone()],
                                originals: HashMap::from([(archive, original)]),
                            }
                        } else {
                            UndoEntry::Copy(vec![archive])
                        });
                        let can_reveal = self.last_started_operation() == Some(context.request_id)
                            && self.validation_generation.get() == context.navigation_generation
                            && self.active_location() == context.origin;
                        emit(BrowserEvent::ArchiveCompleted {
                            select_name: if can_reveal {
                                archive_name
                            } else {
                                String::new()
                            },
                        });
                    }
                    OperationEvent::Cancelled { result, .. } => {
                        let mut affected_locations = context.refresh_locations.clone();
                        affected_locations.extend(result.affected_locations);
                        if job.deleting {
                            self.remove_deleted_locations(&result.completed);
                        }
                        emit(BrowserEvent::OperationCancelled {
                            completed: result.completed.len(),
                            failed: result.failed.len(),
                            not_attempted: result.not_attempted.len(),
                            affected_locations,
                        });
                    }
                    OperationEvent::Failed { message, .. }
                    | OperationEvent::TransferFailed { message, .. } => {
                        self.refresh_columns_at_many(&context.refresh_locations);
                        emit(BrowserEvent::OperationFailed { message });
                    }
                    _ => emit(BrowserEvent::OperationFailed {
                        message: "The background operation returned an unexpected result."
                            .to_owned(),
                    }),
                }
                let load = job.load.take();
                drop(load);
            }
        }
    }
}
