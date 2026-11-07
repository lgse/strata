// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    model::Location,
    services::{
        ItemIdentity, MoveRecord, NameMode, RestoreRequest, RestoreSource, UndoCopyRequest,
        UndoMergeRequest, UndoMoveItem, UndoMoveRequest,
    },
};

use super::super::undo::{
    UndoEntry, claim_pending_undo, claim_replay, finish_replay, finish_undo, retain_merge_records,
    retain_replay_locations, retain_replay_merge_items, retain_replay_move_items,
    undo_move_parents,
};
use super::super::{Browser, BrowserEvent};
use super::Replay;

impl Browser {
    pub fn undo_last_trash(self: &Rc<Self>) -> bool {
        if self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_pending_undo(None) else {
            return false;
        };
        let UndoEntry::Trash {
            locations,
            originals,
        } = &entry
        else {
            finish_undo(generation, false);
            return false;
        };
        let request = (locations.clone(), originals.clone());
        let replay = Replay::undo(generation);
        self.restore_original_locations(replay, entry, request)
    }

    /// Moves completed transfers back. `generation` pins the undo the caller
    /// inspected, so an operation started while conflicts were being confirmed
    /// wins instead of being reverted.
    pub fn undo_move(self: &Rc<Self>, generation: u64, items: Vec<UndoMoveItem>) -> bool {
        self.replay_move(false, generation, items)
    }

    /// Copy undo uses Trash so it never permanently deletes data.
    pub fn undo_copy(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_entry_locations(Replay::undo(generation), UndoEntry::Copy(locations), false)
    }

    fn replay_move(self: &Rc<Self>, redo: bool, generation: u64, items: Vec<UndoMoveItem>) -> bool {
        if items.is_empty() || self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_replay(redo, Some(generation)) else {
            return false;
        };
        let UndoEntry::Move(_) = entry else {
            finish_replay(redo, generation, false);
            return false;
        };
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_replay(redo, generation, false);
            return false;
        };
        retain_replay_move_items(redo, generation, &items);
        let total = items.len();
        let refresh_locations = undo_move_parents(&items);
        let request_id = self.begin_operation();
        self.transfer_operation.set(Some(true));
        self.replay_claim(redo).replace(Some((
            generation,
            UndoEntry::Move(items.iter().map(|item| item.record.clone()).collect()),
        )));
        self.emit(BrowserEvent::TransferStarted {
            total,
            moving: true,
        });
        let load = provider.undo_move(
            UndoMoveRequest {
                id: request_id,
                cleanup_locations: Vec::new(),
                names: if redo { NameMode::Fix } else { NameMode::Keep },
                items: items
                    .into_iter()
                    .map(|item| {
                        if redo {
                            UndoMoveItem {
                                record: MoveRecord {
                                    original: item.record.current,
                                    current: item.record.original,
                                    renamed: Vec::new(),
                                    replaced: item.record.replaced,
                                },
                                conflict: item.conflict,
                            }
                        } else {
                            item
                        }
                    })
                    .collect(),
            },
            self.operation_callback(request_id, false, refresh_locations),
        );
        self.install_operation_load(request_id, load);
        true
    }

    pub fn redo_trash(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_entry_locations(
            Replay::redo(generation),
            UndoEntry::trashed(locations, &HashMap::new()),
            false,
        )
    }

    pub fn redo_copy(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_entry_locations(Replay::redo(generation), UndoEntry::Copy(locations), true)
    }

    /// Replays a copy undo or a trash or copy redo, which list plain locations:
    /// `restore` brings them back from Trash, otherwise they go to Trash.
    fn replay_entry_locations(
        self: &Rc<Self>,
        requested: Replay,
        entry: UndoEntry,
        restore: bool,
    ) -> bool {
        let locations = replayed_locations(&entry);
        if locations.is_empty() || self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, claimed)) = claim_replay(requested.redo, Some(requested.generation))
        else {
            return false;
        };
        let replay = Replay {
            redo: requested.redo,
            generation,
        };
        if std::mem::discriminant(&claimed) != std::mem::discriminant(&entry) {
            replay.finish(false);
            return false;
        }
        if restore {
            self.restore_original_locations(replay, entry, (locations, HashMap::new()))
        } else {
            retain_replay_locations(replay.redo, replay.generation, &locations);
            self.retrash_locations(replay, entry, locations)
        }
    }

    pub fn redo_move(self: &Rc<Self>, generation: u64, items: Vec<UndoMoveItem>) -> bool {
        self.replay_move(true, generation, items)
    }

    fn retrash_locations(
        self: &Rc<Self>,
        replay: Replay,
        entry: UndoEntry,
        locations: Vec<Location>,
    ) -> bool {
        let Some(provider) = self.operation_provider.borrow().clone() else {
            replay.finish(false);
            return false;
        };
        let total = locations.len();
        let refresh_locations = locations
            .iter()
            .filter_map(|location| location.parent())
            .collect();
        let request_id = self.begin_operation();
        self.deletion_operation.set(true);
        self.replay_claim(replay.redo)
            .replace(Some((replay.generation, entry)));
        self.emit(BrowserEvent::DeletionStarted { total });
        let load = provider.undo_copy(
            UndoCopyRequest {
                id: request_id,
                locations,
            },
            self.operation_callback(request_id, false, refresh_locations),
        );
        self.install_operation_load(request_id, load);
        true
    }

    /// Merge undo restores originals from Trash, so it runs through the
    /// restoration pipeline like trash undo.
    pub fn undo_merge(
        self: &Rc<Self>,
        generation: u64,
        created: Vec<Location>,
        overwritten: Vec<Location>,
    ) -> bool {
        if self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_pending_undo(Some(generation)) else {
            return false;
        };
        let UndoEntry::Merge {
            mut originals,
            mut placed,
            ..
        } = entry
        else {
            finish_undo(generation, false);
            return false;
        };
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_undo(generation, false);
            return false;
        };
        retain_replay_merge_items(false, generation, &created, &overwritten);
        originals.retain(|location, _| overwritten.contains(location));
        retain_merge_records(&mut placed, &created, &overwritten);
        let total = created.len() + overwritten.len();
        let refresh_locations = created
            .iter()
            .chain(&overwritten)
            .filter_map(|location| location.parent())
            .collect();
        let request_id = self.begin_operation();
        self.restoration_operation.set(true);
        self.undo_claim.replace(Some((
            generation,
            UndoEntry::Merge {
                created: created.clone(),
                overwritten: overwritten.clone(),
                originals: originals.clone(),
                placed: placed.clone(),
            },
        )));
        self.emit(BrowserEvent::RestorationStarted { total });
        let load = provider.undo_merge(
            UndoMergeRequest {
                id: request_id,
                created,
                overwritten,
                originals,
                placed,
            },
            self.operation_callback(request_id, false, refresh_locations),
        );
        self.install_operation_load(request_id, load);
        true
    }

    fn restore_original_locations(
        self: &Rc<Self>,
        replay: Replay,
        entry: UndoEntry,
        (locations, originals): (Vec<Location>, HashMap<Location, ItemIdentity>),
    ) -> bool {
        let Some(provider) = self.operation_provider.borrow().clone() else {
            replay.finish(false);
            return false;
        };
        let total = locations.len();
        let request_id = self.begin_operation();
        self.restoration_operation.set(true);
        self.replay_claim(replay.redo)
            .replace(Some((replay.generation, entry)));
        self.emit(BrowserEvent::RestorationStarted { total });
        let load = provider.restore(
            RestoreRequest {
                id: request_id,
                source: RestoreSource::OriginalLocations {
                    locations,
                    originals,
                },
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.install_operation_load(request_id, load);
        true
    }
}

fn replayed_locations(entry: &UndoEntry) -> Vec<Location> {
    match entry {
        UndoEntry::Trash { locations, .. } | UndoEntry::Copy(locations) => locations.clone(),
        _ => Vec::new(),
    }
}
