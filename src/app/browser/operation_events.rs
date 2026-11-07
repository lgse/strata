// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    model::Location,
    services::{ItemIdentity, OperationEvent, OperationRequestId, TransferReport},
};

mod background;
mod outcome;
mod progress;
mod replay;
pub(super) use background::BackgroundOperation;
use outcome::{Outcome, replaced_archive_undo};
use replay::finish_claimed_replay;

use super::{
    Browser, BrowserEvent, MergeUndoState, UndoEntry, completed_replay_items, finish_replay,
    mark_replay_item_completed, move_records, moved_target, push_pending_redo, push_pending_undo,
    push_regenerated_undo, record_replaced_originals, retain_merge_records,
};

/// The items a transfer moved, where each landed, and its report.
type Moves<'a> = (
    &'a [Location],
    &'a HashMap<Location, Location>,
    &'a TransferReport,
);

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
    trashed: RefCell<HashMap<Location, ItemIdentity>>,
}

struct OperationCompletion {
    moving: Option<bool>,
    deleting: bool,
    deletion_permanent: bool,
    restoring: bool,
    recovering: bool,
    archiving: bool,
    destination: Option<Location>,
    reveal: bool,
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
            recovering: browser.recovery_operation.replace(false),
            archiving: browser.archive_operation.replace(false),
            destination: browser.transfer_destination.replace(None),
            reveal: browser.transfer_reveal.replace(true),
            undoing: false,
            reveal_locations: Vec::new(),
        }
    }

    /// Whether a deletion leaves anything in Trash that undo can restore.
    fn records_trash_undo(&self) -> bool {
        self.deleting && !self.deletion_permanent && !self.undoing
    }

    fn changes_files(&self) -> bool {
        self.deleting || self.restoring || self.moving.is_some()
    }

    fn reveals(&self) -> bool {
        self.reveal && !self.undoing && !self.reveal_locations.is_empty()
    }

    fn replays_restore(&self) -> bool {
        self.restoring && !self.undoing
    }

    fn record_restore_undo(&self, restored: &[Location]) {
        if self.replays_restore() && !restored.is_empty() {
            push_pending_undo(UndoEntry::Copy(restored.to_vec()));
        }
    }

    fn record_trash_undo(
        &self,
        event: &OperationEvent,
        trashed: &HashMap<Location, ItemIdentity>,
        publish: fn(UndoEntry),
    ) {
        if !self.records_trash_undo() {
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
        publish(UndoEntry::trashed(locations, trashed));
    }

    /// Returns the moved items kept out of undo.
    fn record_transfer_undo(
        &self,
        moves: Moves<'_>,
        created: Vec<Location>,
        (merged, trashed): (MergeUndoState, &HashMap<Location, ItemIdentity>),
        publish: fn(UndoEntry),
    ) -> Vec<Location> {
        if self.undoing {
            return Vec::new();
        }
        match (self.moving, &self.destination) {
            (Some(true), Some(destination)) => {
                record_move_undo(moves, destination, (&merged, trashed), publish)
            }
            (Some(false), _) => {
                record_copy_undo(created, merged, trashed, publish);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

fn record_move_undo(
    (moved, moved_to, report): Moves<'_>,
    destination: &Location,
    (merged, trashed): (&MergeUndoState, &HashMap<Location, ItemIdentity>),
    publish: fn(UndoEntry),
) -> Vec<Location> {
    let kept_back = kept_back(moved, report);
    // A merged move deletes its source, so it cannot be moved
    // back, and moving an item back over what its move left
    // behind would replace newer entries: exclude both.
    let movable: Vec<_> = moved
        .iter()
        .filter(|source| !merged.sources.contains(*source))
        .filter(|source| !kept_back.contains(*source))
        .cloned()
        .collect();
    let mut records = move_records(&movable, destination, moved_to, &report.renamed);
    record_replaced_originals(&mut records, trashed, true);
    publish(UndoEntry::Move(records));
    moved
        .iter()
        .filter(|source| kept_back.contains(source))
        .cloned()
        .collect()
}

fn record_copy_undo(
    created: Vec<Location>,
    merged: MergeUndoState,
    trashed: &HashMap<Location, ItemIdentity>,
    publish: fn(UndoEntry),
) {
    let (merged_created, overwritten, mut placed) = merged.into_records();
    if merged_created.is_empty() && overwritten.is_empty() {
        publish(UndoEntry::Copy(created));
        return;
    }
    let mut all_created = created;
    all_created.extend(merged_created);
    // A replaced target reports as created through transfer
    // progress; it must undo through the overwritten restore
    // path instead, or the restored original would be trashed.
    let replaced = overwritten.iter().collect::<HashSet<_>>();
    all_created.retain(|location| !replaced.contains(location));
    retain_merge_records(&mut placed, &all_created, &overwritten);
    let originals = overwritten
        .iter()
        .filter_map(|location| Some((location.clone(), *trashed.get(location)?)))
        .collect();
    publish(UndoEntry::Merge {
        created: all_created,
        overwritten,
        originals,
        placed,
    });
}

fn kept_back<'a>(moved: &'a [Location], report: &TransferReport) -> HashSet<&'a Location> {
    let moved = moved.iter().collect::<HashSet<_>>();
    let mut kept_back = HashSet::new();
    for kept in &report.kept {
        let mut location = Some(kept.location.clone());
        while let Some(current) = location {
            if let Some(source) = moved.get(&current) {
                kept_back.insert(*source);
            }
            location = current.parent();
        }
    }
    kept_back
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
                trashed: RefCell::default(),
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
            } => {
                changes
                    .trashed
                    .borrow_mut()
                    .insert(location.clone(), *identity);
                self.publish_removal(changes, location, Some(*identity));
            }
            // A merged move staged these overwritten destination items in Trash.
            OperationEvent::Merged { overwritten, .. } if changes.moving => {
                self.publish_removals(changes, overwritten)
            }
            OperationEvent::DeleteProgress {
                deleted_locations, ..
            } => self.publish_removals(changes, deleted_locations),
            OperationEvent::ItemRestored {
                location, identity, ..
            } => self.publish_restoration(changes, location, Some(*identity)),
            OperationEvent::RestoreProgress {
                restored_location: Some(location),
                ..
            } => self.publish_restoration(changes, location, None),
            OperationEvent::Restored { restored, .. }
            | OperationEvent::RestoreCompletedWithErrors { restored, .. } => {
                for location in restored {
                    self.publish_restoration(changes, location, None);
                }
            }
            // Restores also list completed items in these terminals, so only a
            // deletion's terminal names removals its progress did not report.
            _ if changes.deleting => self.publish_removals(changes, &deleted_locations(event)),
            _ => {}
        }
    }

    fn publish_removals(&self, changes: &LocationChanges, locations: &[Location]) {
        for location in locations {
            self.publish_removal(changes, location, None);
        }
    }

    fn publish_removal(
        &self,
        changes: &LocationChanges,
        location: &Location,
        trash_identity: Option<ItemIdentity>,
    ) {
        if changes.removed.borrow_mut().insert(location.clone()) {
            self.emit(BrowserEvent::ItemRemoved {
                location: location.clone(),
                trash_identity,
            });
        }
    }

    fn publish_restoration(
        &self,
        changes: &LocationChanges,
        location: &Location,
        identity: Option<ItemIdentity>,
    ) {
        if changes.restored.borrow_mut().insert(location.clone()) {
            self.emit(BrowserEvent::ItemRestored {
                location: location.clone(),
                identity,
            });
        }
    }

    fn finish_operation(self: &Rc<Self>, context: &OperationContext, event: OperationEvent) {
        self.current_operation.set(None);
        self.transfer_cancel_pending.set(false);
        self.end_rename_operation(context);
        let mut completion = OperationCompletion::take(self);
        self.transfer_report.borrow_mut().follow_renames();
        self.flush_operation_changes(&completion);
        let refresh_depths = self.take_rescan_depths();
        // Flush observers run before the undo claim is consumed, as on the provider path.
        let trashed = context.location_changes.trashed.take();
        completion.undoing = self.finish_replay_claims(&event, &trashed);
        completion.record_trash_undo(&event, &trashed, push_pending_undo);
        self.finish_transfer(&mut completion, &event, &trashed);
        self.publish_finished_removal(&completion, &event);
        let load = self.operation_load.take();
        drop(load);
        let mut report = self.transfer_report.take();
        let outcome = Outcome {
            context,
            completion,
            report: &mut report,
            rescanning: &refresh_depths,
        };
        self.publish_operation_outcome(outcome, event);
        if !report.is_empty() {
            self.emit(BrowserEvent::ItemsReported { report });
        }
        if !refresh_depths.is_empty() {
            self.emit(BrowserEvent::OperationRefreshRequired {
                depths: refresh_depths,
            });
        }
    }

    fn end_rename_operation(&self, context: &OperationContext) {
        if context.rename && self.rename_operation.get() == Some(context.request_id) {
            self.rename_operation.set(None);
        }
    }

    fn take_rescan_depths(&self) -> Vec<usize> {
        let mut depths = self
            .operation_rescan_depths
            .take()
            .into_iter()
            .collect::<Vec<_>>();
        depths.sort_unstable();
        depths
    }

    /// Returns whether the operation was an undo or redo replay.
    fn finish_replay_claims(
        &self,
        event: &OperationEvent,
        trashed: &HashMap<Location, ItemIdentity>,
    ) -> bool {
        let undoing = self.undo_claim.take();
        if let Some((generation, entry)) = &undoing {
            finish_claimed_replay(false, *generation, entry, (event, trashed));
        }
        let redoing = self.redo_claim.take();
        if let Some((generation, entry)) = &redoing {
            finish_claimed_replay(true, *generation, entry, (event, trashed));
        }
        undoing.is_some() || redoing.is_some()
    }

    fn publish_finished_removal(&self, completion: &OperationCompletion, event: &OperationEvent) {
        if completion.deleting {
            self.emit(BrowserEvent::DeletionFinished {
                succeeded: matches!(event, OperationEvent::Deleted { .. }),
            });
        }
        if completion.restoring {
            self.emit(BrowserEvent::RestorationFinished {
                succeeded: matches!(event, OperationEvent::Restored { .. }),
            });
        }
    }

    fn flush_operation_changes(self: &Rc<Self>, completion: &OperationCompletion) {
        if !completion.changes_files() {
            return;
        }
        let changes = self.deferred_file_operation_changes.take();
        self.flush_deferred_file_operation_changes(changes);
    }

    fn finish_transfer(
        self: &Rc<Self>,
        completion: &mut OperationCompletion,
        event: &OperationEvent,
        trashed: &HashMap<Location, ItemIdentity>,
    ) {
        let Some(moving) = completion.moving else {
            return;
        };
        let created = self.created_locations.take();
        let moved_to = self.moved_items.take();
        if let OperationEvent::Pasted { locations, .. } = event {
            completion.reveal_locations = if moving {
                locations
                    .iter()
                    .filter_map(|source| {
                        moved_target(source, completion.destination.as_ref()?, &moved_to)
                    })
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
        let kept_out_of_undo = completion.record_transfer_undo(
            (&moved, &moved_to, &self.transfer_report.borrow()),
            created,
            (self.merged_undo.take(), trashed),
            push_pending_undo,
        );
        self.transfer_report.borrow_mut().kept_out_of_undo = kept_out_of_undo;
        for location in &moved {
            self.retire_recent_target(location);
        }
        self.emit(BrowserEvent::TransferFinished {
            moved_locations: if completion.undoing {
                Vec::new()
            } else {
                moved
            },
        });
    }
}

fn operation_event_id(event: &OperationEvent) -> OperationRequestId {
    match event {
        OperationEvent::Renamed { request_id }
        | OperationEvent::EntryPlaced { request_id, .. }
        | OperationEvent::Created { request_id }
        | OperationEvent::EntryCreated { request_id, .. }
        | OperationEvent::Pasted { request_id, .. }
        | OperationEvent::ItemMoved { request_id, .. }
        | OperationEvent::ItemTrashed { request_id, .. }
        | OperationEvent::ItemRestored { request_id, .. }
        | OperationEvent::ItemKept { request_id, .. }
        | OperationEvent::FlushingToDevice { request_id }
        | OperationEvent::ItemsReported { request_id, .. }
        | OperationEvent::Merged { request_id, .. }
        | OperationEvent::TransferFailed { request_id, .. }
        | OperationEvent::TransferProgress { request_id, .. }
        | OperationEvent::DeleteProgress { request_id, .. }
        | OperationEvent::DeleteEntriesRemoved { request_id, .. }
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
