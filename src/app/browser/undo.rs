// SPDX-License-Identifier: MIT

use std::{cell::RefCell, collections::HashMap};

use crate::{
    model::Location,
    services::{ItemIdentity, MoveRecord, PlacedSnapshot, RenameRecord, UndoMoveItem},
};

mod group;
mod merge;
mod moves;

pub(super) use merge::{MergeUndoState, retain_merge_records};
pub(super) use moves::{move_records, moved_target, record_replaced_originals, undo_move_parents};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UndoEntry {
    Trash {
        locations: Vec<Location>,
        originals: HashMap<Location, ItemIdentity>,
    },
    Move(Vec<MoveRecord>),
    Copy(Vec<Location>),
    /// `created` paths are trashed, or deleted where Trash is unavailable;
    /// `overwritten` paths lose the incoming copy and get their staged
    /// original back from Trash. Either is touched only while it still
    /// matches its `placed` snapshot.
    Merge {
        created: Vec<Location>,
        overwritten: Vec<Location>,
        originals: HashMap<Location, ItemIdentity>,
        placed: HashMap<Location, PlacedSnapshot>,
    },
    Rename(RenameRecord),
    Group {
        folder: Location,
        records: Vec<MoveRecord>,
    },
}

impl UndoEntry {
    pub(super) fn is_empty(&self) -> bool {
        match self {
            Self::Trash { locations, .. } | Self::Copy(locations) => locations.is_empty(),
            Self::Merge {
                created,
                overwritten,
                ..
            } => created.is_empty() && overwritten.is_empty(),
            Self::Move(records) | Self::Group { records, .. } => records.is_empty(),
            Self::Rename(_) => false,
        }
    }

    /// Undoing a copy or merge moves what it placed to Trash, which `admin://` lacks.
    pub(super) fn places_as_administrator(&self) -> bool {
        match self {
            Self::Copy(created) => created.iter().any(Location::is_administrator),
            Self::Merge {
                created,
                overwritten,
                ..
            } => created
                .iter()
                .chain(overwritten)
                .any(Location::is_administrator),
            Self::Trash { .. } | Self::Move(_) | Self::Group { .. } | Self::Rename(_) => false,
        }
    }

    pub(super) fn empty_like(&self) -> Self {
        match self {
            Self::Trash { .. } => Self::trashed(Vec::new(), &HashMap::new()),
            Self::Move(_) => Self::Move(Vec::new()),
            Self::Copy(_) => Self::Copy(Vec::new()),
            Self::Merge { .. } => Self::Merge {
                created: Vec::new(),
                overwritten: Vec::new(),
                originals: HashMap::new(),
                placed: HashMap::new(),
            },
            Self::Rename(record) => Self::Rename(record.clone()),
            Self::Group { folder, .. } => Self::Group {
                folder: folder.clone(),
                records: Vec::new(),
            },
        }
    }

    pub(super) fn locations_mut(&mut self) -> Option<&mut Vec<Location>> {
        match self {
            Self::Trash { locations, .. } | Self::Copy(locations) => Some(locations),
            _ => None,
        }
    }

    pub(super) fn trashed(
        locations: Vec<Location>,
        identities: &HashMap<Location, ItemIdentity>,
    ) -> Self {
        let originals = locations
            .iter()
            .filter_map(|location| Some((location.clone(), *identities.get(location)?)))
            .collect();
        Self::Trash {
            locations,
            originals,
        }
    }
}

pub(super) fn complete_undo_item(
    entry: &mut UndoEntry,
    completed: &mut UndoEntry,
    location: &Location,
    move_source: bool,
) {
    match (entry, completed) {
        (UndoEntry::Move(records), UndoEntry::Move(done))
        | (UndoEntry::Group { records, .. }, UndoEntry::Group { records: done, .. }) => {
            complete_move_record(records, done, location, move_source)
        }
        (
            UndoEntry::Trash { locations, .. } | UndoEntry::Copy(locations),
            UndoEntry::Trash {
                locations: done, ..
            }
            | UndoEntry::Copy(done),
        ) => complete_location(locations, done, location),
        (
            UndoEntry::Merge {
                created,
                overwritten,
                originals,
                placed,
            },
            UndoEntry::Merge {
                created: done_created,
                overwritten: done_overwritten,
                originals: done_originals,
                placed: done_placed,
            },
        ) => {
            complete_location(created, done_created, location);
            complete_location(overwritten, done_overwritten, location);
            complete_keyed(originals, done_originals, location);
            complete_keyed(placed, done_placed, location);
        }
        _ => {}
    }
}

fn complete_move_record(
    records: &mut Vec<MoveRecord>,
    done: &mut Vec<MoveRecord>,
    location: &Location,
    move_source: bool,
) {
    if let Some(index) = records
        .iter()
        .position(|record| move_endpoint(record, move_source) == location)
    {
        done.push(records.remove(index));
    }
}

fn move_endpoint(record: &MoveRecord, move_source: bool) -> &Location {
    if move_source {
        &record.original
    } else {
        &record.current
    }
}

fn complete_location(remaining: &mut Vec<Location>, done: &mut Vec<Location>, location: &Location) {
    if let Some(index) = remaining.iter().position(|candidate| candidate == location) {
        done.push(remaining.remove(index));
    }
}

fn complete_keyed<V>(
    remaining: &mut HashMap<Location, V>,
    done: &mut HashMap<Location, V>,
    location: &Location,
) {
    if let Some(value) = remaining.remove(location) {
        done.insert(location.clone(), value);
    }
}

pub(super) struct PendingUndo {
    pub(super) background: bool,
    pub(super) generation: u64,
    pub(super) entry: UndoEntry,
    /// Items already reversed, kept so a partial undo can offer a matching
    /// redo. Stays `None` until an item completes so single-shot entries like
    /// Rename only redo when they actually reverted.
    pub(super) completed: Option<UndoEntry>,
    pub(super) claimed: bool,
}

pub(super) const MAX_UNDO_HISTORY: usize = 32;

#[derive(Default)]
pub(super) struct UndoState {
    pub(super) next_generation: u64,
    pub(super) history: Vec<PendingUndo>,
    pub(super) redo: Vec<PendingUndo>,
    pub(super) group_folder: Option<Location>,
    pub(super) group_rename_finished: bool,
}

impl UndoState {
    pub(super) fn stack(&self, redo: bool) -> &[PendingUndo] {
        if redo { &self.redo } else { &self.history }
    }

    pub(super) fn stack_mut(&mut self, redo: bool) -> &mut Vec<PendingUndo> {
        if redo {
            &mut self.redo
        } else {
            &mut self.history
        }
    }

    pub(super) fn find_in(stack: &mut [PendingUndo], generation: u64) -> Option<&mut PendingUndo> {
        stack
            .iter_mut()
            .find(|pending| pending.generation == generation)
    }

    pub(super) fn push_entry(&mut self, redo: bool, entry: UndoEntry) {
        let generation = self.next_generation.saturating_add(1);
        self.next_generation = generation;
        let stack = self.stack_mut(redo);
        stack.push(PendingUndo {
            background: false,
            generation,
            completed: None,
            entry,
            claimed: false,
        });
        while stack.len() > MAX_UNDO_HISTORY {
            let Some(index) = stack[..stack.len() - 1]
                .iter()
                .position(|pending| !pending.claimed)
            else {
                break;
            };
            stack.remove(index);
        }
    }
}

// Undo follows the latest operation across every Strata window on the GTK main thread.
thread_local! {
    pub(super) static PENDING_UNDO: RefCell<UndoState> = RefCell::new(UndoState::default());
}

pub(super) fn push_pending_undo(entry: UndoEntry) {
    if entry.is_empty() || entry.places_as_administrator() {
        return;
    }
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        pending.redo.retain(|entry| entry.claimed);
        pending.push_entry(false, entry);
        pending.fold_group_tail();
    });
}

/// Naming a new item moves or replaces the item its creation recorded, so
/// undoing that creation removes what the naming made instead.
pub(super) fn retarget_created_undo(created: &Location, placed: Location) {
    let retargeted = PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        let recorded = pending.history.iter_mut().rev().find(|undo| {
            !undo.claimed
                && matches!(&undo.entry, UndoEntry::Copy(locations)
                    if locations.as_slice() == std::slice::from_ref(created))
        });
        recorded.map(|undo| undo.entry = UndoEntry::Copy(vec![placed.clone()]))
    });
    if retargeted.is_none() {
        push_pending_undo(UndoEntry::Copy(vec![placed]));
    }
}

pub(super) fn push_background_undo(entry: UndoEntry) {
    if entry.is_empty() || entry.places_as_administrator() {
        return;
    }
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        pending.redo.retain(|entry| entry.claimed);
        pending.push_entry(false, entry);
        let mut entry = pending.history.pop().expect("new background undo");
        entry.background = true;
        let anchor = pending.group_folder.as_ref().and_then(|folder| {
            pending
                .history
                .iter()
                .rposition(|entry| !entry.claimed && records_group_folder(&entry.entry, folder))
        });
        let position = anchor.unwrap_or(pending.history.len());
        pending.history.insert(position, entry);
    });
}

/// The entry that created `folder`, or the group it already gathered.
fn records_group_folder(entry: &UndoEntry, folder: &Location) -> bool {
    matches!(entry, UndoEntry::Copy(locations) if locations.as_slice() == std::slice::from_ref(folder))
        || matches!(entry, UndoEntry::Group { folder: grouped, .. } if grouped == folder)
}

pub(super) fn push_regenerated_undo(entry: UndoEntry) {
    if entry.is_empty() {
        return;
    }
    PENDING_UNDO.with(|pending| pending.borrow_mut().push_entry(false, entry));
}

pub(super) fn push_pending_redo(entry: UndoEntry) {
    if entry.is_empty() {
        return;
    }
    PENDING_UNDO.with(|pending| pending.borrow_mut().push_entry(true, entry));
}

pub(super) fn peek_stack(stack: &[PendingUndo]) -> Option<(u64, UndoEntry)> {
    let latest = stack.last()?;
    (!latest.claimed).then(|| (latest.generation, latest.entry.clone()))
}

pub(super) fn claim_stack(
    stack: &mut [PendingUndo],
    expected: Option<u64>,
) -> Option<(u64, UndoEntry)> {
    let latest = stack.last_mut()?;
    if latest.claimed || expected.is_some_and(|generation| generation != latest.generation) {
        return None;
    }
    latest.claimed = true;
    Some((latest.generation, latest.entry.clone()))
}

pub(super) fn finish_stack(stack: &mut Vec<PendingUndo>, generation: u64, completed: bool) {
    let Some(entry) = UndoState::find_in(stack, generation) else {
        return;
    };
    entry.claimed = false;
    if !completed
        && let UndoEntry::Group { folder, records } = &entry.entry
        && records.is_empty()
    {
        entry.entry = UndoEntry::Copy(vec![folder.clone()]);
    }
    if completed || entry.entry.is_empty() {
        stack.retain(|pending| pending.generation != generation);
    }
}

pub(super) fn peek_replay(redo: bool) -> Option<(u64, UndoEntry)> {
    PENDING_UNDO.with(|pending| peek_stack(pending.borrow().stack(redo)))
}

pub(super) fn claim_replay(redo: bool, expected: Option<u64>) -> Option<(u64, UndoEntry)> {
    PENDING_UNDO.with(|pending| claim_stack(pending.borrow_mut().stack_mut(redo), expected))
}

pub(super) fn claim_pending_undo(expected: Option<u64>) -> Option<(u64, UndoEntry)> {
    claim_replay(false, expected)
}

/// A redone move travels from `original`, so its source is what matches events.
pub(super) fn mark_replay_item_completed(redo: bool, generation: u64, location: &Location) {
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        let Some(pending) = UndoState::find_in(pending.stack_mut(redo), generation) else {
            return;
        };
        let mut completed = pending
            .completed
            .take()
            .unwrap_or_else(|| pending.entry.empty_like());
        complete_undo_item(&mut pending.entry, &mut completed, location, redo);
        pending.completed = Some(completed);
    });
}

pub(super) fn completed_replay_items(redo: bool, generation: u64) -> Option<UndoEntry> {
    PENDING_UNDO.with(|pending| {
        pending
            .borrow()
            .stack(redo)
            .iter()
            .find(|pending| pending.generation == generation)
            .and_then(|pending| pending.completed.clone())
    })
}

pub(super) fn finish_replay(redo: bool, generation: u64, completed: bool) {
    PENDING_UNDO
        .with(|pending| finish_stack(pending.borrow_mut().stack_mut(redo), generation, completed));
}

pub(super) fn finish_undo(generation: u64, completed: bool) {
    finish_replay(false, generation, completed);
}

pub(super) fn finish_redo(generation: u64, completed: bool) {
    finish_replay(true, generation, completed);
}

pub(super) fn retain_replay_move_items(redo: bool, generation: u64, items: &[UndoMoveItem]) {
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        if let Some(pending) = UndoState::find_in(pending.stack_mut(redo), generation)
            && let Some(records) = match &mut pending.entry {
                UndoEntry::Move(records) | UndoEntry::Group { records, .. } => Some(records),
                _ => None,
            }
        {
            records.retain(|record| items.iter().any(|item| &item.record == record));
        }
    });
}

pub(super) fn retain_replay_locations(redo: bool, generation: u64, locations: &[Location]) {
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        if let Some(pending) = UndoState::find_in(pending.stack_mut(redo), generation)
            && let Some(recorded) = pending.entry.locations_mut()
        {
            recorded.retain(|location| locations.contains(location));
        }
    });
}

pub(super) fn retain_replay_merge_items(
    redo: bool,
    generation: u64,
    created: &[Location],
    overwritten: &[Location],
) {
    PENDING_UNDO.with(|pending| {
        let mut pending = pending.borrow_mut();
        if let Some(pending) = UndoState::find_in(pending.stack_mut(redo), generation)
            && let UndoEntry::Merge {
                created: kept_created,
                overwritten: kept_overwritten,
                originals,
                placed,
            } = &mut pending.entry
        {
            kept_created.retain(|location| created.contains(location));
            kept_overwritten.retain(|location| overwritten.contains(location));
            originals.retain(|location, _| overwritten.contains(location));
            retain_merge_records(placed, created, overwritten);
        }
    });
}

#[cfg(test)]
pub(super) fn pending_undo_entry() -> Option<UndoEntry> {
    PENDING_UNDO.with(|pending| {
        pending
            .borrow()
            .history
            .last()
            .map(|pending| pending.entry.clone())
    })
}

#[cfg(test)]
pub(super) fn pending_redo_entry() -> Option<UndoEntry> {
    PENDING_UNDO.with(|pending| {
        pending
            .borrow()
            .redo
            .last()
            .map(|pending| pending.entry.clone())
    })
}
