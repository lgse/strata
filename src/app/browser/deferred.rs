// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::services::MetadataOutcome;

use super::{Browser, METADATA_FILL_DEBOUNCE};

impl Browser {
    pub(super) fn schedule_metadata_fill(self: &Rc<Self>) {
        if self.metadata_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = gio::glib::timeout_add_local_once(METADATA_FILL_DEBOUNCE, move || {
            if let Some(browser) = weak.upgrade() {
                browser.flush_metadata_fills();
            }
        });
        *self.metadata_timer.borrow_mut() = Some(source);
    }

    fn cancel_metadata_timer(&self) {
        if let Some(source) = self.metadata_timer.borrow_mut().take() {
            source.remove();
        }
    }

    pub(super) fn truncate_deferred_from(self: &Rc<Self>, len: usize) {
        self.retain_metadata_work(len);
        self.retain_sort_work(len);
        self.retain_load_buffers(len);
    }

    fn retain_metadata_work(self: &Rc<Self>, len: usize) {
        self.cancel_metadata_timer();
        self.metadata_pending
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        if !self.metadata_pending.borrow().is_empty() {
            self.schedule_metadata_fill();
        }
        self.metadata_loads
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        // This borrow must end before retain_sort_work notifies observers.
        let state = self.state.borrow();
        self.fill_tokens.borrow_mut().retain(|_, fill| {
            fill.depth < len
                && state.request_id_for_depth(fill.depth) == Some(fill.directory_request)
        });
    }

    fn retain_sort_work(&self, len: usize) {
        let awaiting = *self.sort_awaiting_fill.borrow();
        if let Some(awaiting) = awaiting
            && awaiting.depth >= len
        {
            self.abandon_awaited_sort(
                awaiting.depth,
                awaiting.generation,
                MetadataOutcome::Cancelled,
            );
        } else {
            self.sort_loads.borrow_mut().retain(|depth, _| *depth < len);
        }
    }

    fn retain_load_buffers(&self, len: usize) {
        self.remote.borrow_mut().retain_depths(len);
        self.last_batch_selection
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        self.staging.borrow_mut().retain(|depth, _| *depth < len);
        self.sorting.borrow_mut().retain(|depth, _| *depth < len);
        self.staged_publishes
            .borrow_mut()
            .retain(|depth, _| *depth < len);
        if self.staged_publishes.borrow().is_empty()
            && let Some(source) = self.publish_timer.borrow_mut().take()
        {
            source.remove();
        }
    }

    /// Discard work only when its load/data source is being replaced wholesale.
    pub(super) fn cancel_deferred_work(&self) {
        self.cancel_metadata_timer();
        self.metadata_pending.borrow_mut().clear();
        self.metadata_loads.borrow_mut().clear();
        self.fill_tokens.borrow_mut().clear();
        self.cancel_sort_work();
        self.remote.borrow_mut().clear();
        self.last_batch_selection.borrow_mut().clear();
        self.staging.borrow_mut().clear();
        self.sorting.borrow_mut().clear();
        self.staged_publishes.borrow_mut().clear();
        if let Some(source) = self.publish_timer.borrow_mut().take() {
            source.remove();
        }
        self.cancel_remote_timer();
    }

    fn cancel_sort_work(&self) {
        let awaiting = self.sort_awaiting_fill.borrow_mut().take();
        if let Some(awaiting) = awaiting {
            self.abandon_awaited_sort(
                awaiting.depth,
                awaiting.generation,
                MetadataOutcome::Cancelled,
            );
        } else {
            self.sort_loads.borrow_mut().clear();
            if let Some((_, depth)) = self.pending_sort.take() {
                self.emit(super::BrowserEvent::SortingFinished { depth });
            }
        }
    }
}
