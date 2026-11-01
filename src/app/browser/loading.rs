// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{
    model::FileEntry,
    services::{DirectoryEvent, RequestId},
};

use super::{Browser, BrowserEvent, directory_changes::DepartureEvidence, remote::RemoteTerminal};

mod metadata;

#[derive(Clone, Copy)]
pub(super) struct LoadCompletion {
    pub(super) truncated: bool,
    pub(super) can_trash: Option<bool>,
    pub(super) can_delete: Option<bool>,
}

enum OpenLoad {
    Native(usize),
    Remote(usize),
}

impl Browser {
    pub(super) fn handle_directory_event(self: &Rc<Self>, event: DirectoryEvent) {
        match event {
            DirectoryEvent::Stamp { request_id, stamp } => {
                self.watch_answered_folder(request_id, Some(&stamp));
                self.record_stamp(request_id, *stamp);
            }
            DirectoryEvent::ListingDigest {
                request_id,
                names_digest,
            } => self.record_digest(request_id, names_digest),
            DirectoryEvent::Batch {
                request_id,
                entries,
            } => {
                self.watch_answered_folder(request_id, None);
                self.receive_batch(request_id, entries);
            }
            DirectoryEvent::Finished {
                request_id,
                truncated,
                can_trash,
                can_delete,
            } => {
                if !truncated {
                    self.watch_answered_folder(request_id, None);
                }
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
                refused,
                denied,
            } => self.receive_failure(request_id, message, refused, denied),
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
        if state.location_at(depth)?.native_path().is_some()
            || self.preserving_refreshes.borrow().contains(&depth)
        {
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
            let mut entries = entries;
            let remainder = if self
                .location_at(depth)
                .is_some_and(|location| location.is_camera_photo_root())
                && entries.len() > super::CAMERA_FLUSH_CAP
            {
                entries.split_off(super::CAMERA_FLUSH_CAP)
            } else {
                Vec::new()
            };
            self.apply_owned_batch(request_id, entries);
            if !remainder.is_empty() {
                self.accumulate_batch(request_id, depth, remainder);
            }
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
        let target = self.open_load_target(request_id);
        if let Some(OpenLoad::Native(depth)) = target
            && completion.truncated
            && self.cut_short_of_a_complete_listing(depth, request_id)
            && self.keep_listing(depth, request_id, false, false)
        {
            return;
        }
        match target {
            Some(OpenLoad::Native(depth)) => {
                self.stage_batch(request_id, depth, Vec::new());
                self.finish_staged_load(depth, request_id, completion);
            }
            Some(OpenLoad::Remote(depth)) => {
                self.remote.borrow_mut().set_terminal(
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

    fn receive_failure(
        self: &Rc<Self>,
        request_id: RequestId,
        message: String,
        refused: bool,
        denied: bool,
    ) {
        let depth = self.state.borrow().depth_for_request(request_id);
        if depth.is_some_and(|depth| self.keep_listing(depth, request_id, true, refused)) {
            return;
        }
        if let Some(depth) = depth {
            self.note_listing_unfinished(depth, refused);
        }
        match self.open_load_target(request_id) {
            Some(OpenLoad::Native(depth)) => {
                self.staging.borrow_mut().remove(&depth);
                self.sorting.borrow_mut().remove(&depth);
                self.cancel_publish(depth);
            }
            Some(OpenLoad::Remote(depth)) => {
                self.remote.borrow_mut().set_terminal(
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
        let administrator = denied
            .then(|| self.state.borrow().depth_for_request(request_id))
            .flatten()
            .and_then(|depth| self.location_at(depth))
            .and_then(|location| self.administrator_alternative(&location));
        let mut state = self.state.borrow_mut();
        if let Some(depth) = state.fail(request_id, message.clone(), administrator.clone()) {
            drop(state);
            self.emit(BrowserEvent::LoadFailed {
                depth,
                message,
                administrator,
            });
            self.check_failed_root_load(depth);
        } else if state.fail_peek(request_id, message.clone()) {
            drop(state);
            self.emit(BrowserEvent::PeekFailed { message });
        }
    }
}

impl Browser {
    fn cut_short_of_a_complete_listing(&self, depth: usize, request_id: RequestId) -> bool {
        let listed = self
            .staging
            .borrow()
            .get(&depth)
            .filter(|staged| staged.request_id == request_id)
            .map_or(0, |staged| staged.entries.len());
        listed < super::MAX_DIRECTORY_ENTRIES
            && self
                .kept_listings
                .borrow()
                .get(&depth)
                .is_some_and(|kept| kept.request_id == request_id && !kept.truncated)
    }

    fn keep_listing(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
        failed: bool,
        refused: bool,
    ) -> bool {
        let kept = self
            .kept_listings
            .borrow()
            .get(&depth)
            .copied()
            .filter(|kept| kept.request_id == request_id);
        let Some(kept) = kept else {
            return false;
        };
        self.kept_listings.borrow_mut().remove(&depth);
        let mut deltas = self
            .staging
            .borrow_mut()
            .remove(&depth)
            .map(|staging| staging.deltas)
            .unwrap_or_default();
        if let Some(sorting) = self.sorting.borrow_mut().remove(&depth) {
            deltas.extend(sorting.deltas);
        }
        self.cancel_publish(depth);
        if !self
            .state
            .borrow_mut()
            .keep_listing(request_id, kept.truncated)
        {
            return false;
        }
        let owed = self.take_owed_refresh(depth);
        self.note_listing_kept(depth);
        self.emit(BrowserEvent::LoadFinished {
            depth,
            truncated: kept.truncated,
        });
        match kept.background {
            true => self.owe_refresh(depth, owed),
            false => self.note_listing_unfinished(depth, refused),
        }
        for (watched, change) in deltas {
            self.handle_directory_change(depth, &watched, change);
        }
        if let Some(location) = self.location_at(depth).filter(|location| {
            failed
                && kept.background
                && !Self::operation_touches(location)
                && !self.departure_check_pending()
        }) {
            self.check_departed_directory(depth, location, DepartureEvidence::FailedLoad);
        }
        self.follow_responsiveness();
        true
    }
}

#[cfg(test)]
mod tests;
