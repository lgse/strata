// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use crate::{
    model::Location,
    services::{ItemIdentity, MoveRecord, PlacedSnapshot, RenameRecord, RenamedItem, UndoMoveItem},
};

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
            if let Some(index) = records.iter().position(|record| {
                let endpoint = if move_source {
                    &record.original
                } else {
                    &record.current
                };
                endpoint == location
            }) {
                done.push(records.remove(index));
            }
        }
        (
            UndoEntry::Trash { locations, .. } | UndoEntry::Copy(locations),
            UndoEntry::Trash {
                locations: done, ..
            }
            | UndoEntry::Copy(done),
        ) => {
            if let Some(index) = locations.iter().position(|candidate| candidate == location) {
                done.push(locations.remove(index));
            }
        }
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
            if let Some(index) = created.iter().position(|candidate| candidate == location) {
                done_created.push(created.remove(index));
            }
            if let Some(index) = overwritten
                .iter()
                .position(|candidate| candidate == location)
            {
                done_overwritten.push(overwritten.remove(index));
            }
            if let Some(original) = originals.remove(location) {
                done_originals.insert(location.clone(), original);
            }
            if let Some(snapshot) = placed.remove(location) {
                done_placed.insert(location.clone(), snapshot);
            }
        }
        _ => {}
    }
}

/// Merge bookkeeping collected while a paste runs: which sources merged, and
/// what each merge created or overwrote. Sources matter for move transfers,
/// where a merged source must not join the move-undo records.
#[derive(Default)]
pub(super) struct MergeUndoState {
    pub(super) sources: HashSet<Location>,
    pub(super) created: Vec<Location>,
    /// `None` once its incoming copy was saved under another name.
    pub(super) overwritten: Vec<Option<Location>>,
    pub(super) created_at: HashMap<Location, Vec<usize>>,
    pub(super) overwritten_at: HashMap<Location, Vec<usize>>,
    pub(super) placed: HashMap<Location, PlacedSnapshot>,
}

impl MergeUndoState {
    pub(super) fn merged(
        &mut self,
        source: Location,
        created: Vec<Location>,
        overwritten: Vec<Location>,
        placed: HashMap<Location, PlacedSnapshot>,
    ) {
        self.sources.insert(source);
        for location in created {
            self.create(location);
        }
        for location in overwritten {
            self.overwritten_at
                .entry(location.clone())
                .or_default()
                .push(self.overwritten.len());
            self.overwritten.push(Some(location));
        }
        self.placed.extend(placed);
    }

    pub(super) fn create(&mut self, location: Location) {
        self.created_at
            .entry(location.clone())
            .or_default()
            .push(self.created.len());
        self.created.push(location);
    }

    /// An overwritten original stays in Trash when its copy landed under
    /// another name, since another item now holds that name. Renames apply in
    /// reported order and are not chained: a record under a planned name
    /// follows only the rename that starts at that name.
    pub(super) fn rename(&mut self, renamed: &[RenamedItem]) {
        for item in renamed {
            if let Some(positions) = self.created_at.remove(&item.requested) {
                for &position in &positions {
                    self.created[position] = item.saved_as.clone();
                }
                self.created_at
                    .entry(item.saved_as.clone())
                    .or_default()
                    .extend(positions);
            }
            let first = self
                .overwritten_at
                .get_mut(&item.requested)
                .filter(|positions| !positions.is_empty())
                .map(|positions| positions.remove(0));
            if let Some(position) = first {
                self.overwritten[position] = None;
                self.create(item.saved_as.clone());
            }
        }
    }

    pub(super) fn into_records(
        self,
    ) -> (
        Vec<Location>,
        Vec<Location>,
        HashMap<Location, PlacedSnapshot>,
    ) {
        let overwritten = self.overwritten.into_iter().flatten().collect();
        (self.created, overwritten, self.placed)
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

    pub(super) fn fold_group_tail(&mut self) {
        let Some(created) = self.group_folder.clone() else {
            return;
        };
        let tail = |offset: usize| {
            self.history
                .iter()
                .rev()
                .nth(offset)
                .filter(|top| !top.claimed)
        };
        enum Fold {
            Fuse {
                folder: Location,
                records: Vec<MoveRecord>,
                done: bool,
            },
            Clear,
        }
        let fold = match tail(0).map(|top| top.entry.clone()) {
            Some(UndoEntry::Move(records))
                if !records.is_empty()
                    && records
                        .iter()
                        .all(|record| record.current.parent().as_ref() == Some(&created))
                    && matches!(
                        tail(1).map(|top| &top.entry),
                        Some(UndoEntry::Copy(locations))
                            if locations.as_slice() == std::slice::from_ref(&created)
                    ) =>
            {
                Fold::Fuse {
                    folder: created,
                    records,
                    done: self.group_rename_finished,
                }
            }
            Some(UndoEntry::Rename(record)) => match tail(1).map(|top| &top.entry) {
                Some(UndoEntry::Group { folder, records }) if *folder == record.original => {
                    let folder = record.current.clone();
                    Fold::Fuse {
                        records: records
                            .iter()
                            .map(|moved| MoveRecord {
                                original: moved.original.clone(),
                                current: moved
                                    .original
                                    .file_name()
                                    .and_then(|name| folder.child(&name))
                                    .unwrap_or_else(|| moved.current.clone()),
                                renamed: Vec::new(),
                            })
                            .collect(),
                        folder,
                        done: true,
                    }
                }
                _ => Fold::Clear,
            },
            _ => Fold::Clear,
        };
        match fold {
            Fold::Fuse {
                folder,
                records,
                done,
            } => {
                let len = self.history.len();
                self.history.truncate(len - 2);
                self.push_entry(false, UndoEntry::Group { folder, records });
                if done {
                    self.group_folder = None;
                }
            }
            Fold::Clear => self.group_folder = None,
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

/// Naming a new item with a path moves or replaces the item its creation
/// recorded, so undoing that creation removes what the naming made instead.
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
            pending.history.iter().rposition(|entry| !entry.claimed && matches!(&entry.entry,
                UndoEntry::Copy(locations) if locations.as_slice() == std::slice::from_ref(folder))
                || !entry.claimed && matches!(&entry.entry, UndoEntry::Group { folder: grouped, .. } if grouped == folder))
        });
        let position = anchor.unwrap_or(pending.history.len());
        pending.history.insert(position, entry);
    });
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

pub(super) fn retain_merge_records(
    placed: &mut HashMap<Location, PlacedSnapshot>,
    created: &[Location],
    overwritten: &[Location],
) {
    let undone: HashSet<&Location> = created.iter().chain(overwritten).collect();
    placed.retain(|location, _| undone.contains(location));
}

pub(super) fn undo_move_parents(items: &[UndoMoveItem]) -> HashSet<Location> {
    items
        .iter()
        .flat_map(|item| [&item.record.current, &item.record.original])
        .filter_map(Location::parent)
        .collect()
}

pub(super) fn moved_target(
    source: &Location,
    destination: &Location,
    moved_to: &HashMap<Location, Location>,
) -> Option<Location> {
    moved_to
        .get(source)
        .cloned()
        .or_else(|| source.transfer_target(destination))
}

/// Items that never left their own directory carry no undo.
pub(super) fn move_records(
    sources: &[Location],
    destination: &Location,
    moved_to: &HashMap<Location, Location>,
    renamed: &[RenamedItem],
) -> Vec<MoveRecord> {
    let mut records = sources
        .iter()
        .filter_map(|source| {
            let current = moved_target(source, destination, moved_to)?;
            (&current != source).then(|| MoveRecord {
                original: source.clone(),
                current,
                renamed: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    let moved = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.current.clone(), index))
        .collect::<HashMap<_, _>>();
    for item in renamed {
        let mut folder = item.saved_as.parent();
        while let Some(location) = folder {
            if let Some(&index) = moved.get(&location) {
                records[index].renamed.push(item.clone());
                break;
            }
            folder = location.parent();
        }
    }
    records
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
