// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    rc::{Rc, Weak},
};

use crate::{model::FileEntry, services::RequestId};

use super::{Browser, BrowserEvent, loading::LoadCompletion};

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

pub(super) struct RemoteState {
    flush_timer: Option<gio::glib::SourceId>,
    terminals: HashMap<usize, RemoteTerminal>,
    pending: HashMap<usize, (RequestId, Vec<FileEntry>)>,
}

impl RemoteState {
    pub(super) fn new() -> Self {
        Self {
            flush_timer: None,
            terminals: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    pub(super) fn queue(
        &mut self,
        request_id: RequestId,
        depth: usize,
        entries: Vec<FileEntry>,
    ) -> bool {
        let slot = self
            .pending
            .entry(depth)
            .or_insert_with(|| (request_id, Vec::new()));
        if slot.0 != request_id {
            *slot = (request_id, Vec::new());
        }
        slot.1.extend(entries);
        slot.1.len() >= super::COALESCE_ENTRIES
    }

    pub(super) fn depths(&self, depth: Option<usize>) -> Vec<usize> {
        match depth {
            Some(depth) => vec![depth],
            None => self.pending.keys().copied().collect(),
        }
    }

    pub(super) fn take_chunk(&mut self, depth: usize) -> Option<(RequestId, Vec<FileEntry>)> {
        self.pending.get_mut(&depth).and_then(|slot| {
            if slot.1.is_empty() {
                return None;
            }
            let take = slot.1.len().min(super::REMOTE_FLUSH_CAP);
            Some((slot.0, slot.1.drain(..take).collect()))
        })
    }

    pub(super) fn has_pending(&self, depth: usize) -> bool {
        self.pending.contains_key(&depth)
    }

    pub(super) fn prune_pending(&mut self) {
        self.pending.retain(|_, (_, entries)| !entries.is_empty());
    }

    pub(super) fn has_any_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    #[cfg(test)]
    pub(super) fn has_no_work(&self) -> bool {
        self.pending.is_empty() && self.terminals.is_empty()
    }

    pub(super) fn retain_depths(&mut self, len: usize) {
        self.pending.retain(|depth, _| *depth < len);
        self.terminals.retain(|depth, _| *depth < len);
    }

    pub(super) fn clear(&mut self) {
        self.pending.clear();
        self.terminals.clear();
    }

    pub(super) fn clear_depth(&mut self, depth: usize) {
        self.pending.remove(&depth);
        self.terminals.remove(&depth);
    }

    pub(super) fn set_terminal(&mut self, depth: usize, terminal: RemoteTerminal) {
        self.terminals.insert(depth, terminal);
    }

    pub(super) fn take_terminal(&mut self, depth: usize) -> Option<RemoteTerminal> {
        self.terminals.remove(&depth)
    }

    pub(super) fn take_timer(&mut self) -> Option<gio::glib::SourceId> {
        self.flush_timer.take()
    }

    pub(super) fn timer_armed(&self) -> bool {
        self.flush_timer.is_some()
    }

    pub(super) fn set_timer(&mut self, source: gio::glib::SourceId) {
        self.flush_timer = Some(source);
    }
}

impl Browser {
    pub(super) fn accumulate_batch(
        self: &Rc<Self>,
        request_id: RequestId,
        depth: usize,
        entries: Vec<FileEntry>,
    ) {
        let full = self.remote.borrow_mut().queue(request_id, depth, entries);
        if full {
            self.flush_coalesced_capped(Some(depth));
        } else {
            self.arm_remote_flush_timer();
        }
    }

    pub(super) fn flush_coalesced_capped(self: &Rc<Self>, depth: Option<usize>) {
        let depths = self.remote.borrow().depths(depth);
        for &depth in &depths {
            self.drain_publish(depth);
            let chunk = self.remote.borrow_mut().take_chunk(depth);
            if let Some((request_id, entries)) = chunk {
                self.apply_owned_batch(request_id, entries);
            }
        }
        self.remote.borrow_mut().prune_pending();
        let timer = {
            let mut remote = self.remote.borrow_mut();
            if !remote.has_any_pending() {
                remote.take_timer()
            } else {
                None
            }
        };
        if let Some(source) = timer {
            source.remove();
        } else if self.remote.borrow().has_any_pending() {
            self.arm_remote_flush_timer();
        }
        for depth in depths {
            self.finish_remote_if_drained(depth);
        }
    }

    fn finish_remote_if_drained(self: &Rc<Self>, depth: usize) {
        let terminal = {
            let mut remote = self.remote.borrow_mut();
            if remote.has_pending(depth) {
                return;
            }
            remote.take_terminal(depth)
        };
        let Some(terminal) = terminal else { return };
        match terminal {
            RemoteTerminal::Finished {
                request_id,
                completion:
                    LoadCompletion {
                        truncated,
                        can_trash,
                        can_delete,
                    },
            } => {
                let finished = self
                    .state
                    .borrow_mut()
                    .finish(request_id, truncated, can_trash, can_delete);
                if let Some(depth) = finished {
                    self.emit(BrowserEvent::LoadFinished { depth, truncated });
                    self.ensure_sorted_after_load(depth);
                }
            }
            RemoteTerminal::Failed {
                request_id,
                message,
            } => {
                let failed = self.state.borrow_mut().fail(request_id, message.clone());
                if let Some(depth) = failed {
                    self.emit(BrowserEvent::LoadFailed { depth, message });
                }
            }
        }
    }

    fn arm_remote_flush_timer(self: &Rc<Self>) {
        if self.remote.borrow().timer_armed() {
            return;
        }
        let weak: Weak<Self> = Rc::downgrade(self);
        let source = gio::glib::timeout_add_local_once(super::REMOTE_FLUSH_DELAY, move || {
            if let Some(browser) = weak.upgrade() {
                // Disarm before flushing: a fired source cannot be removed.
                browser.remote.borrow_mut().take_timer();
                browser.flush_coalesced_capped(None);
            }
        });
        self.remote.borrow_mut().set_timer(source);
    }

    pub(super) fn cancel_remote_timer(&self) {
        if let Some(source) = self.remote.borrow_mut().take_timer() {
            source.remove();
        }
    }
}

#[cfg(test)]
mod tests;
