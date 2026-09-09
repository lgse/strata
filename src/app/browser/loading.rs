// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{
    model::FileEntry,
    services::{DirectoryEvent, RequestId},
};

use super::{Browser, BrowserEvent};

mod metadata;

#[derive(Clone, Copy)]
pub(super) struct LoadCompletion {
    pub(super) truncated: bool,
    pub(super) can_trash: Option<bool>,
    pub(super) can_delete: Option<bool>,
}

pub(super) enum RemoteTerminal {
    Finished {
        request_id: RequestId,
        completion: LoadCompletion,
    },
    Failed {
        request_id: RequestId,
        message: String,
    },
}

enum OpenLoad {
    Native(usize),
    Remote(usize),
}

impl Browser {
    pub(super) fn handle_directory_event(self: &Rc<Self>, event: DirectoryEvent) {
        match event {
            DirectoryEvent::Batch {
                request_id,
                entries,
            } => self.receive_batch(request_id, entries),
            DirectoryEvent::Finished {
                request_id,
                truncated,
                can_trash,
                can_delete,
            } => {
                self.receive_completion(
                    request_id,
                    LoadCompletion {
                        truncated,
                        can_trash,
                        can_delete,
                    },
                );
            }
            DirectoryEvent::MetadataIncomplete { request_id } => {
                self.receive_incomplete_metadata(request_id)
            }
            DirectoryEvent::Failed {
                request_id,
                message,
            } => self.receive_failure(request_id, message),
            DirectoryEvent::MetadataFilled {
                request_id,
                updates,
            } => self.receive_metadata(request_id, updates),
            DirectoryEvent::MetadataFinished {
                request_id,
                outcome,
            } => self.handle_metadata_finished(request_id, outcome),
        }
    }

    fn open_load_target(&self, request_id: RequestId) -> Option<OpenLoad> {
        // Return owned routing information so no state borrow survives a flush
        // or synchronous observer callback that navigates to another location.
        let state = self.state.borrow();
        let depth = state.depth_for_request(request_id)?;
        state.open_load_depth(request_id)?;
        if state.location_at(depth)?.native_path().is_some() {
            Some(OpenLoad::Native(depth))
        } else {
            Some(OpenLoad::Remote(depth))
        }
    }

    fn receive_batch(self: &Rc<Self>, request_id: RequestId, entries: Vec<FileEntry>) {
        match self.open_load_target(request_id) {
            Some(OpenLoad::Native(depth)) => self.stage_batch(request_id, depth, entries),
            Some(OpenLoad::Remote(depth)) => self.receive_remote_batch(request_id, depth, entries),
            None => self.receive_peek_batch(request_id, entries),
        }
    }

    fn receive_remote_batch(
        self: &Rc<Self>,
        request_id: RequestId,
        depth: usize,
        entries: Vec<FileEntry>,
    ) {
        let entry_count = self
            .state
            .borrow()
            .loading_column(request_id)
            .map(|(_, count)| count)
            .unwrap_or(0);
        if entry_count == 0 {
            self.apply_owned_batch(request_id, entries);
        } else {
            self.accumulate_batch(request_id, depth, entries);
        }
    }

    fn receive_peek_batch(&self, request_id: RequestId, entries: Vec<FileEntry>) {
        let entries = if self.preferences.get().show_hidden {
            entries
        } else {
            entries
                .into_iter()
                .filter(|entry| !entry.is_hidden)
                .collect()
        };
        let applied = self
            .state
            .borrow_mut()
            .apply_peek_batch(request_id, &entries);
        if applied {
            self.emit(BrowserEvent::PeekEntriesAdded { entries });
        }
    }

    fn receive_completion(self: &Rc<Self>, request_id: RequestId, completion: LoadCompletion) {
        match self.open_load_target(request_id) {
            Some(OpenLoad::Native(depth)) => {
                self.stage_batch(request_id, depth, Vec::new());
                self.finish_staged_load(depth, request_id, completion);
            }
            Some(OpenLoad::Remote(depth)) => {
                self.remote_terminals.borrow_mut().insert(
                    depth,
                    RemoteTerminal::Finished {
                        request_id,
                        completion,
                    },
                );
                self.flush_coalesced_capped(Some(depth));
            }
            None => {
                let finished = self.state.borrow_mut().finish_peek(request_id);
                if finished {
                    self.emit(BrowserEvent::PeekFinished);
                }
            }
        }
    }

    fn receive_incomplete_metadata(self: &Rc<Self>, request_id: RequestId) {
        let Some(OpenLoad::Native(depth)) = self.open_load_target(request_id) else {
            return;
        };
        self.stage_batch(request_id, depth, Vec::new());
        if let Some(staging) = self.staging.borrow_mut().get_mut(&depth) {
            staging.metadata_incomplete = true;
        }
    }

    fn receive_failure(self: &Rc<Self>, request_id: RequestId, message: String) {
        match self.open_load_target(request_id) {
            Some(OpenLoad::Native(depth)) => {
                self.staging.borrow_mut().remove(&depth);
                self.sorting.borrow_mut().remove(&depth);
                self.cancel_publish(depth);
            }
            Some(OpenLoad::Remote(depth)) => {
                self.remote_terminals.borrow_mut().insert(
                    depth,
                    RemoteTerminal::Failed {
                        request_id,
                        message,
                    },
                );
                self.flush_coalesced_capped(Some(depth));
                return;
            }
            None => {}
        }
        let mut state = self.state.borrow_mut();
        if let Some(depth) = state.fail(request_id, message.clone()) {
            drop(state);
            self.emit(BrowserEvent::LoadFailed { depth, message });
        } else if state.fail_peek(request_id, message.clone()) {
            drop(state);
            self.emit(BrowserEvent::PeekFailed { message });
        }
    }
}

#[cfg(test)]
mod tests;
