// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    model::Location,
    services::{OperationEvent, OperationRequestId, TrashedOriginal},
};

mod background;
pub(super) use background::BackgroundOperation;

use super::{
    Browser, BrowserEvent, MergeUndoState, UndoEntry, completed_replay_items, finish_replay,
    mark_replay_item_completed, move_records, push_pending_redo, push_pending_undo,
    push_regenerated_undo,
};

struct OperationContext {
    request_id: OperationRequestId,
    rename: bool,
    refresh_locations: HashSet<Location>,
    navigation_generation: u64,
    origin: Option<Location>,
    location_changes: LocationChanges,
}

/// Per-request state for publishing path changes as each item lands, even
/// after a later operation superseded this one.
struct LocationChanges {
    deleting: bool,
    moving: bool,
    removed: RefCell<HashSet<Location>>,
    restored: RefCell<HashSet<Location>>,
}

struct OperationCompletion {
    moving: Option<bool>,
    deleting: bool,
    deletion_permanent: bool,
    restoring: bool,
    archiving: bool,
    destination: Option<Location>,
    reveal: bool,
    file_operation_refreshed: bool,
    undoing: bool,
    reveal_locations: Vec<Location>,
}

impl OperationCompletion {
    fn take(browser: &Browser) -> Self {
        Self {
            moving: browser.transfer_operation.replace(None),
            deleting: browser.deletion_operation.replace(false),
            deletion_permanent: browser.deletion_permanent.replace(false),
            restoring: browser.restoration_operation.replace(false),
            archiving: browser.archive_operation.replace(false),
            destination: browser.transfer_destination.replace(None),
            reveal: browser.transfer_reveal.replace(true),
            file_operation_refreshed: false,
            undoing: false,
            reveal_locations: Vec::new(),
        }
    }

    fn record_trash_undo(&self, event: &OperationEvent, publish: fn(UndoEntry)) {
        if !self.deleting || self.deletion_permanent || self.undoing {
            return;
        }
        let locations = match event {
            OperationEvent::Deleted { locations, .. } => locations.clone(),
            OperationEvent::CompletedWithErrors {
                deleted_locations, ..
            } => deleted_locations.clone(),
            OperationEvent::Cancelled { result, .. } => result.completed.clone(),
            _ => Vec::new(),
        };
        publish(UndoEntry::Trash(locations));
    }

    fn record_transfer_undo(
        &self,
        moved: &[Location],
        created: Vec<Location>,
        merged: MergeUndoState,
        publish: fn(UndoEntry),
    ) {
        if self.undoing {
            return;
        }
        match self.moving {
            Some(true) => {
                if let Some(destination) = &self.destination {
                    // A merged move deletes its source, so it cannot be moved
                    // back: exclude merged sources from the move records.
                    let movable: Vec<_> = moved
                        .iter()
                        .filter(|source| !merged.sources.contains(*source))
                        .cloned()
                        .collect();
                    publish(UndoEntry::Move(move_records(&movable, destination)));
                }
            }
            Some(false) if merged.overwritten.is_empty() && merged.created.is_empty() => {
                publish(UndoEntry::Copy(created))
            }
            Some(false) => {
                let mut all_created = created;
                all_created.extend(merged.created);
                // A replaced target reports as created through transfer
                // progress; it must undo through the overwritten restore
                // path instead, or the restored original would be trashed.
                all_created.retain(|location| !merged.overwritten.contains(location));
                publish(UndoEntry::Merge {
                    created: all_created,
                    overwritten: merged.overwritten,
                    originals: HashMap::new(),
                });
            }
            None => {}
        }
    }
}

impl Browser {
    pub(super) fn operation_callback(
        self: &Rc<Self>,
        request_id: OperationRequestId,
        rename: bool,
        refresh_locations: HashSet<Location>,
    ) -> Rc<dyn Fn(OperationEvent)> {
        let context = OperationContext {
            request_id,
            rename,
            refresh_locations,
            navigation_generation: self.validation_generation.get(),
            origin: self.active_location(),
            location_changes: LocationChanges {
                deleting: self.deletion_operation.get(),
                moving: self.transfer_operation.get() == Some(true),
                removed: RefCell::default(),
                restored: RefCell::default(),
            },
        };
        let weak = Rc::downgrade(self);
        Rc::new(move |event| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            let event_id = operation_event_id(&event);
            if event_id != context.request_id {
                return;
            }
            browser.track_location_changes(&context.location_changes, &event);
            let background = browser
                .background_operations
                .borrow()
                .get(&event_id)
                .cloned();
            if let Some(job) = background {
                browser.handle_background_operation(&context, event, job);
                return;
            }
            if !browser.is_current_operation(event_id) {
                return;
            }
            if !browser.publish_operation_progress(&event) {
                browser.finish_operation(&context, event);
            }
        })
    }

    fn track_location_changes(&self, changes: &LocationChanges, event: &OperationEvent) {
        match event {
            OperationEvent::ItemMoved {
                from, to, merged, ..
            } => self.emit(BrowserEvent::ItemRelocated {
                from: from.clone(),
                to: to.clone(),
                merged: *merged,
            }),
            OperationEvent::ItemTrashed {
                location, identity, ..
            } => self.publish_removal(changes, location, Some(*identity)),
            // A merged move staged these overwritten destination items in Trash.
            OperationEvent::Merged { overwritten, .. } if changes.moving => {
                for location in overwritten {
                    self.publish_removal(changes, location, None);
                }
            }
            OperationEvent::DeleteProgress {
                deleted_locations, ..
            } => {
                for location in deleted_locations {
                    self.publish_removal(changes, location, None);
                }
            }
            OperationEvent::RestoreProgress {
                restored_location: Some(location),
                ..
            } => self.publish_restoration(changes, location),
            OperationEvent::Restored { restored, .. }
            | OperationEvent::RestoreCompletedWithErrors { restored, .. } => {
                for location in restored {
                    self.publish_restoration(changes, location);
                }
            }
            // Restores also list completed items in these terminals, so only a
            // deletion's terminal names removals its progress did not report.
            _ if changes.deleting => {
                for location in deleted_locations(event) {
                    self.publish_removal(changes, &location, None);
                }
            }
            _ => {}
        }
    }

    fn publish_removal(
        &self,
        changes: &LocationChanges,
        location: &Location,
        trash_identity: Option<TrashedOriginal>,
    ) {
        if changes.removed.borrow_mut().insert(location.clone()) {
            self.emit(BrowserEvent::ItemRemoved {
                location: location.clone(),
                trash_identity,
            });
        }
    }

    fn publish_restoration(&self, changes: &LocationChanges, location: &Location) {
        if changes.restored.borrow_mut().insert(location.clone()) {
            self.emit(BrowserEvent::ItemRestored {
                location: location.clone(),
            });
        }
    }

    fn publish_operation_progress(self: &Rc<Self>, event: &OperationEvent) -> bool {
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
                    self.created_locations.borrow_mut().push(location.clone());
                }
                BrowserEvent::TransferProgress {
                    completed_items: *completed_items,
                    completed_files: *completed_files,
                    total_files: *total_files,
                    current_file: current_file.clone(),
                    transferred_bytes: *transferred_bytes,
                    total_bytes: *total_bytes,
                }
            }
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
            OperationEvent::Merged {
                source,
                created,
                overwritten,
                ..
            } => {
                let mut merged = self.merged_undo.borrow_mut();
                merged.sources.insert(source.clone());
                merged.created.extend(created.iter().cloned());
                merged.overwritten.extend(overwritten.iter().cloned());
                return true;
            }
            OperationEvent::ItemMoved { .. } | OperationEvent::ItemTrashed { .. } => return true,
            OperationEvent::ArchiveStarted { total, .. } => {
                BrowserEvent::ArchiveStarted { total: *total }
            }
            OperationEvent::ArchiveProgress {
                completed, total, ..
            } => BrowserEvent::ArchiveProgress {
                completed: *completed,
                total: *total,
            },
            _ => return false,
        };
        if matches!(
            event,
            OperationEvent::DeleteProgress { .. }
                | OperationEvent::TransferProgress { .. }
                | OperationEvent::RestoreProgress { .. }
        ) {
            self.flush_visible_operation_changes();
        }
        self.emit(progress);
        true
    }

    fn mark_restored_undo_item(&self, completed: usize, restored: Option<&Location>) {
        let Some(restored) = restored else {
            return;
        };
        for (claim, redo) in [(&self.undo_claim, false), (&self.redo_claim, true)] {
            match claim.borrow().as_ref() {
                // A trash undo reports the trash item, so completion maps by
                // request order onto the recorded locations.
                Some((generation, UndoEntry::Trash(locations))) => {
                    if let Some(location) = completed
                        .checked_sub(1)
                        .and_then(|index| locations.get(index))
                    {
                        mark_replay_item_completed(redo, *generation, location);
                        return;
                    }
                }
                // Merge undos and copy redos restore to the real destination
                // the event reports directly.
                Some((generation, UndoEntry::Merge { .. } | UndoEntry::Copy(_))) => {
                    mark_replay_item_completed(redo, *generation, restored);
                    return;
                }
                _ => {}
            }
        }
    }

    /// DeleteProgress carries the processed path, which is how a merge undo
    /// reports trashed `created` items.
    fn mark_merged_undo_item_completed(&self, location: &Location) {
        let claim = self.undo_claim.borrow();
        if let Some((generation, UndoEntry::Merge { .. })) = claim.as_ref() {
            mark_replay_item_completed(false, *generation, location);
        }
    }

    fn finish_operation(self: &Rc<Self>, context: &OperationContext, event: OperationEvent) {
        self.current_operation.set(None);
        self.transfer_cancel_pending.set(false);
        if context.rename && self.rename_operation.get() == Some(context.request_id) {
            self.rename_operation.set(None);
        }
        let mut completion = OperationCompletion::take(self);
        completion.file_operation_refreshed = self.flush_operation_changes(&completion);
        let mut refresh_depths = self
            .operation_rescan_depths
            .take()
            .into_iter()
            .collect::<Vec<_>>();
        refresh_depths.sort_unstable();
        completion.file_operation_refreshed |= !refresh_depths.is_empty();
        // Flush observers run before the undo claim is consumed, as on the provider path.
        let undoing = self.undo_claim.take();
        if let Some((generation, entry)) = &undoing {
            finish_claimed_replay(false, *generation, entry, &event);
        }
        let redoing = self.redo_claim.take();
        if let Some((generation, entry)) = &redoing {
            finish_claimed_replay(true, *generation, entry, &event);
        }
        completion.undoing = undoing.is_some() || redoing.is_some();
        completion.record_trash_undo(&event, push_pending_undo);
        let replayed = match (&undoing, &redoing) {
            (Some((_, entry)), _) => Some((false, entry)),
            (_, Some((_, entry))) => Some((true, entry)),
            _ => None,
        };
        self.finish_transfer(&mut completion, &event, replayed);
        if completion.deleting {
            let removed = deleted_locations(&event);
            if !removed.is_empty() {
                self.emit(BrowserEvent::LocationsRemoved { locations: removed });
            }
            self.emit(BrowserEvent::DeletionFinished {
                succeeded: matches!(&event, OperationEvent::Deleted { .. }),
            });
        }
        if completion.restoring {
            self.emit(BrowserEvent::RestorationFinished {
                succeeded: matches!(&event, OperationEvent::Restored { .. }),
            });
        }
        let load = self.operation_load.take();
        drop(load);
        self.publish_operation_outcome(context, completion, event);
        if !refresh_depths.is_empty() {
            self.emit(BrowserEvent::OperationRefreshRequired {
                depths: refresh_depths,
            });
        }
    }

    fn flush_operation_changes(self: &Rc<Self>, completion: &OperationCompletion) -> bool {
        if !completion.deleting && !completion.restoring && completion.moving.is_none() {
            return false;
        }
        let changes = self.deferred_file_operation_changes.take();
        self.flush_deferred_file_operation_changes(changes, false)
    }

    /// `replayed` is the undo (`false`) or redo (`true`) entry this operation replays.
    fn finish_transfer(
        self: &Rc<Self>,
        completion: &mut OperationCompletion,
        event: &OperationEvent,
        replayed: Option<(bool, &UndoEntry)>,
    ) {
        let Some(moving) = completion.moving else {
            return;
        };
        let created = self.created_locations.take();
        if let OperationEvent::Pasted { locations, .. } = event {
            completion.reveal_locations = if moving {
                locations
                    .iter()
                    .filter_map(|source| source.transfer_target(completion.destination.as_ref()?))
                    .collect()
            } else {
                created.clone()
            };
        }
        let moved = if moving {
            moved_locations(event)
        } else {
            Vec::new()
        };
        self.state
            .borrow_mut()
            .retain_selectionless_removals(moved.iter().cloned());
        completion.record_transfer_undo(
            &moved,
            created,
            self.merged_undo.take(),
            push_pending_undo,
        );
        for location in &moved {
            self.retire_recent_target(location);
        }
        let relocations = match replayed {
            Some((redo, entry)) => replayed_relocations(redo, entry, &moved),
            None => completion
                .destination
                .as_ref()
                .map(|destination| {
                    moved
                        .iter()
                        .filter_map(|source| {
                            Some((source.clone(), source.transfer_target(destination)?))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        if !relocations.is_empty() {
            self.emit(BrowserEvent::LocationsRelocated { moves: relocations });
        }
        self.emit(BrowserEvent::TransferFinished {
            moved_locations: if completion.undoing {
                Vec::new()
            } else {
                moved
            },
        });
    }

    fn publish_operation_outcome(
        self: &Rc<Self>,
        context: &OperationContext,
        completion: OperationCompletion,
        event: OperationEvent,
    ) {
        match event {
            OperationEvent::Failed { message, .. } if context.rename => {
                self.emit(BrowserEvent::RenameFailed {
                    request_id: Some(context.request_id),
                    message,
                });
            }
            OperationEvent::Failed {
                message,
                password_failure,
                ..
            } => self.emit(BrowserEvent::OperationFailed {
                message,
                password_failure,
            }),
            OperationEvent::TransferFailed { message, .. } => {
                self.refresh_columns_at_many(&context.refresh_locations);
                self.emit(BrowserEvent::OperationFailed {
                    message,
                    password_failure: None,
                });
            }
            OperationEvent::CompletedWithErrors {
                deleted_locations,
                retryable_locations,
                has_non_retryable_failures,
                message,
                ..
            } => {
                self.remove_completed_locations(&completion, &deleted_locations);
                self.emit(BrowserEvent::OperationCompletedWithErrors {
                    message,
                    retryable_locations,
                    has_non_retryable_failures,
                });
            }
            OperationEvent::Deleted { locations, .. } => {
                self.remove_completed_locations(&completion, &locations);
            }
            OperationEvent::Restored {
                locations,
                restored,
                ..
            } => {
                if completion.restoring && !completion.undoing && !restored.is_empty() {
                    push_pending_undo(UndoEntry::Copy(restored));
                }
                self.remove_completed_locations(&completion, &locations);
            }
            OperationEvent::RestoreCompletedWithErrors {
                restored_locations,
                restored,
                message,
                ..
            } => {
                if completion.restoring && !completion.undoing && !restored.is_empty() {
                    push_pending_undo(UndoEntry::Copy(restored));
                }
                self.remove_completed_locations(&completion, &restored_locations);
                self.emit(BrowserEvent::OperationCompletedWithErrors {
                    message,
                    retryable_locations: Vec::new(),
                    has_non_retryable_failures: true,
                });
            }
            OperationEvent::Cancelled { result, .. } => {
                self.publish_cancelled_operation(context, completion, result)
            }
            OperationEvent::Renamed { .. } => {
                self.emit(BrowserEvent::RenameCompleted {
                    request_id: context.request_id,
                });
                self.refresh_unmonitored_operation_locations(context);
            }
            OperationEvent::Compressed {
                archive_name,
                archive,
                original,
                ..
            } => {
                if !completion.undoing {
                    push_pending_undo(if let Some(original) = original {
                        UndoEntry::Merge {
                            created: Vec::new(),
                            overwritten: vec![archive.clone()],
                            originals: HashMap::from([(archive, original)]),
                        }
                    } else {
                        UndoEntry::Copy(vec![archive])
                    });
                }
                self.emit(BrowserEvent::ArchiveCompleted {
                    select_name: archive_name,
                });
            }
            OperationEvent::Extracted { first_name, .. } => {
                self.emit(BrowserEvent::ArchiveCompleted {
                    select_name: first_name.unwrap_or_default(),
                });
            }
            OperationEvent::Pasted { .. } => self.publish_completed_transfer(context, completion),
            OperationEvent::EntryCreated { location, .. } => {
                if !completion.undoing {
                    push_pending_undo(UndoEntry::Copy(vec![location.clone()]));
                }
                if self.validation_generation.get() == context.navigation_generation {
                    self.emit(BrowserEvent::EntryCreated { location });
                }
                self.refresh_unmonitored_operation_locations(context);
            }
            OperationEvent::Created { .. } => self.refresh_unmonitored_operation_locations(context),
            OperationEvent::TransferProgress { .. }
            | OperationEvent::DeleteProgress { .. }
            | OperationEvent::RestoreProgress { .. }
            | OperationEvent::Merged { .. }
            | OperationEvent::ItemMoved { .. }
            | OperationEvent::ItemTrashed { .. }
            | OperationEvent::ArchiveStarted { .. }
            | OperationEvent::ArchiveProgress { .. }
            | OperationEvent::FlushingToDevice { .. } => {}
        }
    }

    fn remove_completed_locations(
        self: &Rc<Self>,
        completion: &OperationCompletion,
        locations: &[Location],
    ) {
        if !completion.file_operation_refreshed {
            self.remove_deleted_locations(locations);
        }
    }

    fn refresh_unmonitored_operation_locations(self: &Rc<Self>, context: &OperationContext) {
        // Remote locations have no monitor to publish the authoritative change.
        self.refresh_columns_at_many(
            context
                .refresh_locations
                .iter()
                .filter(|location| location.native_path().is_none()),
        );
    }

    fn publish_completed_transfer(
        self: &Rc<Self>,
        context: &OperationContext,
        completion: OperationCompletion,
    ) {
        self.refresh_unmonitored_operation_locations(context);
        if self.can_reveal_completed_transfer(context, &completion)
            && let Some(destination) = completion.destination
        {
            self.emit(BrowserEvent::TransferReveal {
                destination,
                locations: completion.reveal_locations,
            });
        }
        self.emit(BrowserEvent::TransferCompleted);
    }

    fn can_reveal_completed_transfer(
        &self,
        context: &OperationContext,
        completion: &OperationCompletion,
    ) -> bool {
        if !completion.reveal || completion.undoing || completion.reveal_locations.is_empty() {
            return false;
        }
        self.validation_generation.get() == context.navigation_generation
            && self.active_location() == context.origin
    }

    fn publish_cancelled_operation(
        &self,
        context: &OperationContext,
        completion: OperationCompletion,
        result: crate::services::CancelledOperation,
    ) {
        if context.rename {
            self.emit(BrowserEvent::RenameAbandoned {
                request_id: context.request_id,
            });
        }
        let affected_locations = if completion.file_operation_refreshed {
            HashSet::new()
        } else {
            let mut locations = context.refresh_locations.clone();
            locations.extend(result.affected_locations);
            locations
        };
        if completion.restoring && !completion.undoing && !result.completed.is_empty() {
            push_pending_undo(UndoEntry::Copy(result.completed.clone()));
        }
        if completion.archiving {
            self.emit(BrowserEvent::ArchiveCompleted {
                select_name: String::new(),
            });
        }
        self.emit(BrowserEvent::OperationCancelled {
            completed: result.completed.len(),
            failed: result.failed.len(),
            not_attempted: result.not_attempted.len(),
            affected_locations,
        });
    }
}

fn operation_event_id(event: &OperationEvent) -> OperationRequestId {
    match event {
        OperationEvent::Renamed { request_id }
        | OperationEvent::Created { request_id }
        | OperationEvent::EntryCreated { request_id, .. }
        | OperationEvent::Pasted { request_id, .. }
        | OperationEvent::ItemMoved { request_id, .. }
        | OperationEvent::ItemTrashed { request_id, .. }
        | OperationEvent::FlushingToDevice { request_id }
        | OperationEvent::Merged { request_id, .. }
        | OperationEvent::TransferFailed { request_id, .. }
        | OperationEvent::TransferProgress { request_id, .. }
        | OperationEvent::DeleteProgress { request_id, .. }
        | OperationEvent::RestoreProgress { request_id, .. }
        | OperationEvent::Deleted { request_id, .. }
        | OperationEvent::CompletedWithErrors { request_id, .. }
        | OperationEvent::Restored { request_id, .. }
        | OperationEvent::RestoreCompletedWithErrors { request_id, .. }
        | OperationEvent::Failed { request_id, .. }
        | OperationEvent::Compressed { request_id, .. }
        | OperationEvent::Extracted { request_id, .. }
        | OperationEvent::ArchiveStarted { request_id, .. }
        | OperationEvent::Cancelled { request_id, .. }
        | OperationEvent::ArchiveProgress { request_id, .. } => *request_id,
    }
}

fn moved_locations(event: &OperationEvent) -> Vec<Location> {
    match event {
        OperationEvent::Pasted { locations, .. } => locations.clone(),
        OperationEvent::Cancelled { result, .. } => result.completed.clone(),
        OperationEvent::TransferFailed {
            completed_locations,
            ..
        } => completed_locations.clone(),
        _ => Vec::new(),
    }
}

/// Undo moves each record from `current` back to `original`, and redo the reverse.
fn replayed_relocations(
    redo: bool,
    entry: &UndoEntry,
    moved: &[Location],
) -> Vec<(Location, Location)> {
    let (UndoEntry::Move(records) | UndoEntry::Group { records, .. }) = entry else {
        return Vec::new();
    };
    let moved: HashSet<&Location> = moved.iter().collect();
    records
        .iter()
        .map(|record| {
            if redo {
                (&record.original, &record.current)
            } else {
                (&record.current, &record.original)
            }
        })
        .filter(|(from, _)| moved.contains(from))
        .map(|(from, to)| (from.clone(), to.clone()))
        .collect()
}

fn deleted_locations(event: &OperationEvent) -> Vec<Location> {
    match event {
        OperationEvent::Deleted { locations, .. } => locations.clone(),
        OperationEvent::CompletedWithErrors {
            deleted_locations, ..
        } => deleted_locations.clone(),
        OperationEvent::Cancelled { result, .. } => result.completed.clone(),
        _ => Vec::new(),
    }
}

fn finish_claimed_replay(redo: bool, generation: u64, entry: &UndoEntry, event: &OperationEvent) {
    // A restore replay marks each landed item from progress events; a delete
    // or move replay marks from the locations the finish event reports. A
    // merge undo does both: originals mark through progress while its trashed
    // `created` items arrive in the finish event.
    let progress_marked = matches!(
        (entry, redo),
        (UndoEntry::Trash(_), false) | (UndoEntry::Copy(_), true)
    );
    let completed = match entry {
        _ if progress_marked => Vec::new(),
        UndoEntry::Move(_) | UndoEntry::Group { .. } => moved_locations(event),
        UndoEntry::Rename(_) => Vec::new(),
        _ => deleted_locations(event),
    };
    for location in &completed {
        mark_replay_item_completed(redo, generation, location);
    }
    let restoring = matches!(
        (entry, redo),
        (UndoEntry::Trash(_) | UndoEntry::Merge { .. }, false) | (UndoEntry::Copy(_), true)
    );
    let succeeded = match entry {
        _ if restoring => matches!(event, OperationEvent::Restored { .. }),
        UndoEntry::Move(_) | UndoEntry::Group { .. } => {
            matches!(event, OperationEvent::Pasted { .. })
        }
        UndoEntry::Rename(_) => matches!(event, OperationEvent::Renamed { .. }),
        _ => matches!(event, OperationEvent::Deleted { .. }),
    };
    let applied = if succeeded {
        Some(entry.clone())
    } else {
        completed_replay_items(redo, generation).filter(|completed| !completed.is_empty())
    };
    finish_replay(redo, generation, succeeded);
    let Some(applied) = applied else {
        return;
    };
    if redo {
        push_regenerated_undo(applied);
    } else if !matches!(applied, UndoEntry::Merge { .. } | UndoEntry::Group { .. }) {
        // A merge undo mixes deletes and restores, and a group undo a move
        // plus a trash, that no single redo operation can replay.
        push_pending_redo(applied);
    }
}
