// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use crate::{
    model::Location,
    services::{ItemIdentity, OperationEvent},
};

use super::{
    UndoEntry, completed_replay_items, deleted_locations, finish_replay,
    mark_replay_item_completed, moved_locations, push_pending_redo, push_regenerated_undo,
    record_replaced_originals,
};

pub(super) fn finish_claimed_replay(
    redo: bool,
    generation: u64,
    entry: &UndoEntry,
    (event, trashed): (&OperationEvent, &HashMap<Location, ItemIdentity>),
) {
    for location in &replay_completed_locations(redo, entry, event) {
        mark_replay_item_completed(redo, generation, location);
    }
    let succeeded = replay_succeeded(redo, entry, event);
    let applied = if succeeded {
        Some(entry.clone())
    } else {
        completed_replay_items(redo, generation).filter(|completed| !completed.is_empty())
    };
    finish_replay(redo, generation, succeeded);
    if let Some(applied) = applied {
        push_applied_replay(redo, applied, trashed);
    }
}

/// A restore replay marks each landed item from progress events; a delete
/// or move replay marks from the locations the finish event reports. A
/// merge undo does both: originals mark through progress while its trashed
/// `created` items arrive in the finish event.
fn replay_completed_locations(
    redo: bool,
    entry: &UndoEntry,
    event: &OperationEvent,
) -> Vec<Location> {
    let progress_marked = matches!(
        (entry, redo),
        (UndoEntry::Trash { .. }, false) | (UndoEntry::Copy(_), true)
    );
    match entry {
        _ if progress_marked => Vec::new(),
        UndoEntry::Move(_) | UndoEntry::Group { .. } => moved_locations(event),
        UndoEntry::Rename(_) => Vec::new(),
        _ => deleted_locations(event),
    }
}

fn replay_succeeded(redo: bool, entry: &UndoEntry, event: &OperationEvent) -> bool {
    let restoring = matches!(
        (entry, redo),
        (UndoEntry::Trash { .. } | UndoEntry::Merge { .. }, false) | (UndoEntry::Copy(_), true)
    );
    match entry {
        _ if restoring => matches!(event, OperationEvent::Restored { .. }),
        UndoEntry::Move(_) | UndoEntry::Group { .. } => {
            matches!(event, OperationEvent::Pasted { .. })
        }
        UndoEntry::Rename(_) => matches!(event, OperationEvent::Renamed { .. }),
        _ => matches!(event, OperationEvent::Deleted { .. }),
    }
}

fn push_applied_replay(
    redo: bool,
    mut applied: UndoEntry,
    trashed: &HashMap<Location, ItemIdentity>,
) {
    if let UndoEntry::Move(records) = &mut applied {
        record_replaced_originals(records, trashed, redo);
    }
    if redo {
        push_regenerated_undo(match applied {
            UndoEntry::Trash { locations, .. } => UndoEntry::trashed(locations, trashed),
            applied => applied,
        });
    } else if !matches!(applied, UndoEntry::Merge { .. } | UndoEntry::Group { .. }) {
        // A merge undo mixes deletes and restores, and a group undo a move
        // plus a trash, that no single redo operation can replay.
        push_pending_redo(applied);
    }
}
