// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{
    model::{FileEntry, Location},
    services::{OperationEvent, OperationRequestId, RenameRecord, UndoRenameRequest},
};

use super::super::Browser;
use super::super::undo::{UndoEntry, claim_replay, finish_replay};

/// A rename being reverted, or redone, from `source` back to `target`.
struct ReplayedRename {
    redo: bool,
    source: Location,
    target: Location,
    record: RenameRecord,
}

impl ReplayedRename {
    fn new(record: RenameRecord, redo: bool) -> Self {
        let (source, target) = if redo {
            (record.original.clone(), record.current.clone())
        } else {
            (record.current.clone(), record.original.clone())
        };
        Self {
            redo,
            source,
            target,
            record,
        }
    }

    /// A redo names the entry after its target; an undo restores its recorded names.
    fn rename_entry(&self, renamed: &mut FileEntry) {
        renamed.location = self.target.clone();
        if self.redo {
            if let Some(name) = self.target.file_name() {
                renamed.is_hidden = name.to_string_lossy().starts_with('.');
                renamed.native_name = name;
            }
            renamed.display_name = self.target.display_name();
        } else {
            renamed.native_name = self.record.native_name.clone();
            renamed.display_name = self.record.display_name.clone();
            renamed.is_hidden = self.record.is_hidden;
        }
    }
}

impl Browser {
    pub fn undo_rename(self: &Rc<Self>, generation: u64) -> bool {
        self.replay_rename(false, generation)
    }

    pub fn redo_rename(self: &Rc<Self>, generation: u64) -> bool {
        self.replay_rename(true, generation)
    }

    fn replay_rename(self: &Rc<Self>, redo: bool, generation: u64) -> bool {
        if self.current_operation.get().is_some() {
            return false;
        }
        let Some((generation, entry)) = claim_replay(redo, Some(generation)) else {
            return false;
        };
        let UndoEntry::Rename(record) = entry else {
            finish_replay(redo, generation, false);
            return false;
        };
        let Some(provider) = self.operation_provider.borrow().clone() else {
            finish_replay(redo, generation, false);
            return false;
        };
        let refresh_locations = [record.current.parent(), record.original.parent()]
            .into_iter()
            .flatten()
            .collect();
        let request_id = self.begin_operation();
        self.replay_claim(redo)
            .replace(Some((generation, UndoEntry::Rename(record.clone()))));
        let rename = Rc::new(ReplayedRename::new(record, redo));
        let emit = self.operation_callback(request_id, true, refresh_locations);
        let publish = self.publish_replayed_rename(request_id, rename.clone(), emit);
        let load = provider.undo_rename(
            UndoRenameRequest {
                id: request_id,
                current: rename.source.clone(),
                original: rename.target.clone(),
            },
            publish,
        );
        self.install_operation_load(request_id, load);
        true
    }

    /// Publishes the renamed entry once the provider reports the rename, then
    /// forwards every event to the operation's own callback.
    fn publish_replayed_rename(
        self: &Rc<Self>,
        request_id: OperationRequestId,
        rename: Rc<ReplayedRename>,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> Rc<dyn Fn(OperationEvent)> {
        let weak = Rc::downgrade(self);
        Rc::new(move |event: OperationEvent| {
            if matches!(&event, OperationEvent::Renamed { request_id: id } if *id == request_id)
                && let Some(browser) = weak.upgrade()
            {
                browser.follow_replayed_rename(request_id, &rename);
            }
            emit(event);
        })
    }

    fn follow_replayed_rename(
        self: &Rc<Self>,
        request_id: OperationRequestId,
        rename: &ReplayedRename,
    ) {
        self.publish_relocation(&rename.source, &rename.target);
        if !self.is_current_operation(request_id) {
            return;
        }
        match self.entry_at_location(&rename.source) {
            Some(mut renamed) => {
                rename.rename_entry(&mut renamed);
                self.publish_rename(&rename.source, renamed);
            }
            None => self.follow_renamed_location(&rename.source, &rename.target),
        }
    }
}
