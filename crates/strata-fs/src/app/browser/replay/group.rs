// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{
    model::Location,
    services::{NameMode, UndoMoveItem, UndoMoveRequest},
};

use super::super::undo::{
    PENDING_UNDO, PendingUndo, UndoEntry, UndoState, claim_replay, finish_replay,
    retain_replay_move_items, undo_move_parents,
};
use super::super::{Browser, BrowserEvent};

impl Browser {
    pub fn expect_group_folder(&self, created: Location) {
        PENDING_UNDO.with(|pending| {
            let mut pending = pending.borrow_mut();
            if let Some(anchor) = pending
                .history
                .iter()
                .rposition(|entry| created_in_foreground(entry, &created))
            {
                lift_past_background(&mut pending.history, anchor);
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

    fn dispatch_group_undo(
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
}

fn created_in_foreground(pending: &PendingUndo, folder: &Location) -> bool {
    !pending.claimed
        && !pending.background
        && matches!(&pending.entry, UndoEntry::Copy(locations)
            if locations.as_slice() == std::slice::from_ref(folder))
}

/// Moves the folder's creation past background entries recorded after it.
fn lift_past_background(history: &mut [PendingUndo], mut anchor: usize) {
    while anchor + 1 < history.len() && history[anchor + 1].background {
        history.swap(anchor, anchor + 1);
        anchor += 1;
    }
}
