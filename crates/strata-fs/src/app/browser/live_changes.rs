// SPDX-License-Identifier: MIT

//! Coalesces bursts of live directory changes into batches.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use crate::{model::Location, services::DirectoryChange};

use super::{Browser, DeferredDirectoryChanges};

/// Each single live change costs a pass over its column, so a large copy into
/// an open folder would cost the square of its size. Changes apply one at a
/// time until their columns' entries within one burst add up to this budget.
const LIVE_CHANGE_BUDGET: usize = 100_000;
/// Changes closer together than this belong to the same burst.
const LIVE_CHANGE_GAP: Duration = Duration::from_millis(100);
const LIVE_CHANGE_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct LiveChanges {
    /// When the burst's last change arrived, and the entries it has cost so far.
    burst: Cell<Option<(Instant, usize)>>,
    pub(super) batching: Cell<bool>,
    flushing: Cell<bool>,
    pub(super) timer: RefCell<Option<gio::glib::SourceId>>,
}

/// A pending flush must not outlive its browser: its closure is bound to this
/// thread, and another thread iterating the default context would fire it.
impl Drop for LiveChanges {
    fn drop(&mut self) {
        if let Some(source) = self.timer.get_mut().take() {
            source.remove();
        }
    }
}

impl Browser {
    /// Once a monitor's changes outgrow `LIVE_CHANGE_BUDGET`, every live change
    /// queues behind them, keeping their order, until a flush interval passes quietly.
    pub(super) fn spend_live_change_budget(&self, depth: usize) {
        let live = &self.live_changes;
        let counted = !live.flushing.get() && !live.batching.get();
        if counted && !self.live_change_within_budget(depth) {
            live.batching.set(true);
        }
    }

    pub(super) fn queues_live_change(self: &Rc<Self>) -> bool {
        let live = &self.live_changes;
        if live.flushing.get() || !live.batching.get() {
            return false;
        }
        self.schedule_live_change_flush();
        true
    }

    fn live_change_within_budget(&self, depth: usize) -> bool {
        let cost = self
            .state
            .borrow()
            .columns
            .get(depth)
            .map_or(1, |column| column.entries.len().max(1));
        let now = Instant::now();
        let spent = self
            .live_changes
            .burst
            .get()
            .filter(|(last, _)| now.duration_since(*last) < LIVE_CHANGE_GAP)
            .map_or(0, |(_, spent)| spent)
            .saturating_add(cost);
        self.live_changes.burst.set(Some((now, spent)));
        spent <= LIVE_CHANGE_BUDGET
    }

    pub(super) fn defer_directory_change(
        &self,
        depth: usize,
        watched: Location,
        change: DirectoryChange,
    ) {
        self.deferred_file_operation_changes
            .borrow_mut()
            .entry(depth)
            .or_default()
            .push((watched, change));
    }

    fn schedule_live_change_flush(self: &Rc<Self>) {
        if self.live_changes.timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = gio::glib::timeout_add_local_once(LIVE_CHANGE_FLUSH_INTERVAL, move || {
            if let Some(browser) = weak.upgrade() {
                browser.live_changes.timer.take();
                browser.flush_live_changes();
            }
        });
        self.live_changes.timer.replace(Some(source));
    }

    fn flush_live_changes(self: &Rc<Self>) {
        let live = &self.live_changes;
        let queued = if self.file_operation_defers_changes() {
            // The operation publishes the queue with its own changes.
            DeferredDirectoryChanges::new()
        } else {
            self.deferred_file_operation_changes.take()
        };
        if queued.is_empty() {
            live.batching.set(false);
            live.burst.set(None);
            return;
        }
        self.apply_queued_live_changes(queued);
        self.schedule_live_change_flush();
    }

    /// The burst queued so far lands before a starting operation clears the queue, and
    /// before a rescan reloads a column.
    pub(super) fn publish_queued_live_changes(&self) {
        let live = &self.live_changes;
        if !live.batching.get() {
            return;
        }
        if let Some(source) = live.timer.borrow_mut().take() {
            source.remove();
        }
        live.batching.set(false);
        live.burst.set(None);
        if let Some(browser) = self.self_weak.upgrade() {
            let queued = self.deferred_file_operation_changes.take();
            browser.apply_queued_live_changes(queued);
        }
    }

    fn apply_queued_live_changes(self: &Rc<Self>, queued: DeferredDirectoryChanges) {
        let nested = self.live_changes.flushing.replace(true);
        let ready = self.stage_loading_changes(queued);
        self.flush_deferred_file_operation_changes(ready);
        self.live_changes.flushing.set(nested);
    }

    pub(super) fn flushing_live_changes(&self) -> bool {
        self.live_changes.flushing.get()
    }

    /// Columns still loading record the changes the way a single live change would.
    fn stage_loading_changes(&self, queued: DeferredDirectoryChanges) -> DeferredDirectoryChanges {
        queued
            .into_iter()
            .filter_map(|(depth, changes)| {
                let ready: Vec<_> = changes
                    .into_iter()
                    .filter_map(|(watched, change)| {
                        self.queue_loading_change(depth, &watched, change)
                            .map(|change| (watched, change))
                    })
                    .collect();
                (!ready.is_empty()).then_some((depth, ready))
            })
            .collect()
    }
}
