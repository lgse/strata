// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    model::Location,
    services::{
        ItemIdentity, MoveRecord, NameMode, OperationEvent, RenameRecord, RestoreRequest,
        RestoreSource, UndoCopyRequest, UndoMergeRequest, UndoMoveItem, UndoMoveRequest,
        UndoRenameRequest,
    },
};

use super::undo::{
    PENDING_UNDO, UndoEntry, UndoState, claim_pending_undo, claim_replay, finish_replay,
    finish_undo, peek_replay, retain_merge_records, retain_replay_locations,
    retain_replay_merge_items, retain_replay_move_items, undo_move_parents,
};
use super::{Browser, BrowserEvent};

impl Browser {
    pub(super) fn pending_replay_entry(&self, redo: bool) -> Option<(u64, UndoEntry)> {
        if self.current_operation.get().is_some() {
            return None;
        }
        peek_replay(redo)
    }

    pub fn pending_undo_move(&self) -> Option<(u64, Vec<MoveRecord>)> {
        match self.pending_replay_entry(false)? {
            (generation, UndoEntry::Move(records)) => Some((generation, records)),
            _ => None,
        }
    }

    pub fn pending_undo_copy(&self) -> Option<(u64, Vec<Location>)> {
        match self.pending_replay_entry(false)? {
            (generation, UndoEntry::Copy(locations)) => Some((generation, locations)),
            _ => None,
        }
    }

    pub fn pending_undo_merge(&self) -> Option<(u64, Vec<Location>, Vec<Location>)> {
        match self.pending_replay_entry(false)? {
            (
                generation,
                UndoEntry::Merge {
                    created,
                    overwritten,
                    ..
                },
            ) => Some((generation, created, overwritten)),
            _ => None,
        }
    }

    pub fn pending_undo_rename(&self) -> Option<(u64, Location, Location)> {
        match self.pending_replay_entry(false)? {
            (
                generation,
                UndoEntry::Rename(RenameRecord {
                    original, current, ..
                }),
            ) => Some((generation, current, original)),
            _ => None,
        }
    }

    pub fn pending_undo_trash(&self) -> Option<Vec<Location>> {
        match self.pending_replay_entry(false)? {
            (_, UndoEntry::Trash { locations, .. }) => Some(locations),
            _ => None,
        }
    }

    pub fn pending_undo_group(&self) -> Option<(u64, Vec<MoveRecord>)> {
        match self.pending_replay_entry(false)? {
            (generation, UndoEntry::Group { records, .. }) => Some((generation, records)),
            _ => None,
        }
    }

    pub fn expect_group_folder(&self, created: Location) {
        PENDING_UNDO.with(|pending| {
            let mut pending = pending.borrow_mut();
            if let Some(mut anchor) = pending.history.iter().rposition(|entry| !entry.claimed && !entry.background && matches!(&entry.entry, UndoEntry::Copy(locations) if locations.as_slice() == std::slice::from_ref(&created))) {
                while anchor + 1 < pending.history.len() && pending.history[anchor + 1].background {
                    pending.history.swap(anchor, anchor + 1);
                    anchor += 1;
                }
            }
            pending.group_folder = Some(created);
            pending.group_rename_finished = false;
        });
    }

    pub fn clear_group_folder(&self) {
        PENDING_UNDO.with(|pending| {
            let mut pending = pending.borrow_mut();
            pending.group_rename_finished = true;
            if matches!(pending.history.last().map(|entry| &entry.entry), Some(UndoEntry::Group { folder, .. }) if Some(folder) == pending.group_folder.as_ref()) {
                pending.group_folder = None;
            }
        });
    }

    pub fn clear_group_folder_for(&self, location: &Location) {
        if PENDING_UNDO.with(|pending| pending.borrow().group_folder.as_ref() == Some(location)) {
            self.clear_group_folder();
        }
    }

    pub fn discard_pending_replay(&self, redo: bool, generation: u64) {
        if claim_replay(redo, Some(generation)).is_some() {
            finish_replay(redo, generation, true);
        }
    }

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
        self.restore_original_locations(false, generation, entry, request)
    }

    /// Moves completed transfers back. `generation` pins the undo the caller
    /// inspected, so an operation started while conflicts were being confirmed
    /// wins instead of being reverted.
    pub fn undo_move(self: &Rc<Self>, generation: u64, items: Vec<UndoMoveItem>) -> bool {
        self.replay_move(false, generation, items)
    }

    /// Copy undo uses Trash so it never permanently deletes data.
    pub fn undo_copy(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_entry_locations(
            false,
            generation,
            locations.clone(),
            UndoEntry::Copy(locations),
            false,
        )
    }

    pub fn undo_rename(self: &Rc<Self>, generation: u64) -> bool {
        self.replay_rename(false, generation)
    }

    pub fn undo_group(self: &Rc<Self>, generation: u64, items: Vec<UndoMoveItem>) -> bool {
        if items.is_empty() || self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_replay(false, Some(generation)) else {
            return false;
        };
        let UndoEntry::Group { folder, records } = entry else {
            finish_replay(false, generation, false);
            return false;
        };
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_replay(false, generation, false);
            return false;
        };
        if items.len() != records.len() {
            PENDING_UNDO.with(|pending| {
                if let Some(pending) =
                    UndoState::find_in(&mut pending.borrow_mut().history, generation)
                {
                    pending.entry = UndoEntry::Move(records);
                    pending.claimed = false;
                }
            });
            return self.undo_move(generation, items);
        }
        drop(provider);
        self.dispatch_group_undo(generation, folder, items)
    }

    pub fn undo_group_retaining_folder(
        self: &Rc<Self>,
        generation: u64,
        items: Vec<UndoMoveItem>,
    ) -> bool {
        if items.is_empty() || self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_replay(false, Some(generation)) else {
            return false;
        };
        let UndoEntry::Group { folder, .. } = entry else {
            finish_replay(false, generation, false);
            return false;
        };
        if self.operation_provider.borrow().is_none() {
            finish_replay(false, generation, false);
            return false;
        }
        self.dispatch_group_undo(generation, folder, items)
    }

    pub fn discard_group_keep_folder(&self, redo: bool, generation: u64) {
        let Some((generation, entry)) = claim_replay(redo, Some(generation)) else {
            return;
        };
        if let UndoEntry::Group { folder, .. } = entry {
            PENDING_UNDO.with(|pending| {
                if let Some(pending) =
                    UndoState::find_in(pending.borrow_mut().stack_mut(redo), generation)
                {
                    pending.entry = UndoEntry::Copy(vec![folder]);
                    pending.claimed = false;
                }
            });
        } else {
            finish_replay(redo, generation, true);
        }
    }

    pub(super) fn dispatch_group_undo(
        self: &Rc<Self>,
        generation: u64,
        folder: Location,
        items: Vec<UndoMoveItem>,
    ) -> bool {
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_replay(false, generation, false);
            return false;
        };
        retain_replay_move_items(false, generation, &items);
        let total = items.len();
        let mut refresh_locations = undo_move_parents(&items);
        refresh_locations.extend(folder.parent());
        let request_id = self.begin_operation();
        self.transfer_operation.set(Some(true));
        self.replay_claim(false).replace(Some((
            generation,
            UndoEntry::Group {
                folder: folder.clone(),
                records: items.iter().map(|item| item.record.clone()).collect(),
            },
        )));
        self.emit(BrowserEvent::TransferStarted {
            total,
            moving: true,
        });
        let load = provider.undo_move(
            UndoMoveRequest {
                id: request_id,
                items,
                cleanup_locations: vec![folder],
                names: NameMode::Keep,
            },
            self.operation_callback(request_id, false, refresh_locations),
        );
        self.install_operation_load(request_id, load);
        true
    }

    pub(super) fn replay_move(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        items: Vec<UndoMoveItem>,
    ) -> bool {
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

    pub(super) fn replay_rename(self: &Rc<Self>, redo: bool, generation: u64) -> bool {
        if self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_replay(redo, Some(generation)) else {
            return false;
        };
        let UndoEntry::Rename(RenameRecord {
            original,
            current,
            native_name,
            display_name,
            is_hidden,
        }) = entry
        else {
            finish_replay(redo, generation, false);
            return false;
        };
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_replay(redo, generation, false);
            return false;
        };
        let refresh_locations = [current.parent(), original.parent()]
            .into_iter()
            .flatten()
            .collect();
        let request_id = self.begin_operation();
        self.replay_claim(redo).replace(Some((
            generation,
            UndoEntry::Rename(RenameRecord {
                original: original.clone(),
                current: current.clone(),
                native_name: native_name.clone(),
                display_name: display_name.clone(),
                is_hidden,
            }),
        )));
        let (source, target) = if redo {
            (original, current)
        } else {
            (current, original)
        };
        let source_for_publish = source.clone();
        let target_for_publish = target.clone();
        let weak = Rc::downgrade(self);
        let emit = self.operation_callback(request_id, true, refresh_locations);
        let publish = Rc::new(move |event: OperationEvent| {
            if matches!(&event, OperationEvent::Renamed { request_id: id } if *id == request_id)
                && let Some(browser) = weak.upgrade()
            {
                browser.publish_relocation(&source_for_publish, &target_for_publish);
                if browser.is_current_operation(request_id) {
                    if let Some(mut renamed) = browser.entry_at_location(&source_for_publish) {
                        renamed.location = target_for_publish.clone();
                        if redo {
                            if let Some(name) = target_for_publish.file_name() {
                                renamed.is_hidden = name.to_string_lossy().starts_with('.');
                                renamed.native_name = name;
                            }
                            renamed.display_name = target_for_publish.display_name();
                        } else {
                            renamed.native_name = native_name.clone();
                            renamed.display_name = display_name.clone();
                            renamed.is_hidden = is_hidden;
                        }
                        browser.publish_rename(&source_for_publish, renamed);
                    } else {
                        browser.follow_renamed_location(&source_for_publish, &target_for_publish);
                    }
                }
            }
            emit(event);
        });
        let load = provider.undo_rename(
            UndoRenameRequest {
                id: request_id,
                current: source,
                original: target,
            },
            publish,
        );
        self.install_operation_load(request_id, load);
        true
    }

    pub fn pending_redo_move(&self) -> Option<(u64, Vec<MoveRecord>)> {
        match self.pending_replay_entry(true)? {
            (generation, UndoEntry::Move(records)) => Some((generation, records)),
            _ => None,
        }
    }

    pub fn pending_redo_copy(&self) -> Option<(u64, Vec<Location>)> {
        match self.pending_replay_entry(true)? {
            (generation, UndoEntry::Copy(locations)) => Some((generation, locations)),
            _ => None,
        }
    }

    pub fn pending_redo_rename(&self) -> Option<(u64, Location, Location)> {
        match self.pending_replay_entry(true)? {
            (
                generation,
                UndoEntry::Rename(RenameRecord {
                    original, current, ..
                }),
            ) => Some((generation, original, current)),
            _ => None,
        }
    }

    pub fn pending_redo_trash(&self) -> Option<(u64, Vec<Location>)> {
        match self.pending_replay_entry(true)? {
            (generation, UndoEntry::Trash { locations, .. }) => Some((generation, locations)),
            _ => None,
        }
    }

    pub fn redo_trash(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_entry_locations(
            true,
            generation,
            locations.clone(),
            UndoEntry::trashed(locations, &HashMap::new()),
            false,
        )
    }

    pub fn redo_copy(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_entry_locations(
            true,
            generation,
            locations.clone(),
            UndoEntry::Copy(locations),
            true,
        )
    }

    pub(super) fn replay_entry_locations(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        locations: Vec<Location>,
        entry: UndoEntry,
        restore: bool,
    ) -> bool {
        if locations.is_empty() || self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, claimed)) = claim_replay(redo, Some(generation)) else {
            return false;
        };
        if std::mem::discriminant(&claimed) != std::mem::discriminant(&entry) {
            finish_replay(redo, generation, false);
            return false;
        }
        if restore {
            self.restore_original_locations(redo, generation, entry, (locations, HashMap::new()))
        } else {
            retain_replay_locations(redo, generation, &locations);
            self.retrash_locations(redo, generation, entry, locations)
        }
    }

    pub fn redo_move(self: &Rc<Self>, generation: u64, items: Vec<UndoMoveItem>) -> bool {
        self.replay_move(true, generation, items)
    }

    pub fn redo_rename(self: &Rc<Self>, generation: u64) -> bool {
        self.replay_rename(true, generation)
    }

    pub(super) fn retrash_locations(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        entry: UndoEntry,
        locations: Vec<Location>,
    ) -> bool {
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_replay(redo, generation, false);
            return false;
        };
        let total = locations.len();
        let refresh_locations = locations
            .iter()
            .filter_map(|location| location.parent())
            .collect();
        let request_id = self.begin_operation();
        self.deletion_operation.set(true);
        self.replay_claim(redo).replace(Some((generation, entry)));
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

    pub(super) fn restore_original_locations(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        entry: UndoEntry,
        (locations, originals): (Vec<Location>, HashMap<Location, ItemIdentity>),
    ) -> bool {
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_replay(redo, generation, false);
            return false;
        };
        let total = locations.len();
        let request_id = self.begin_operation();
        self.restoration_operation.set(true);
        self.replay_claim(redo).replace(Some((generation, entry)));
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

    pub(super) fn replay_claim(&self, redo: bool) -> &RefCell<Option<(u64, UndoEntry)>> {
        if redo {
            &self.redo_claim
        } else {
            &self.undo_claim
        }
    }
}
