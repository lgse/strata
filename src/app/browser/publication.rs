// SPDX-License-Identifier: MIT

use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use gio::glib;

use super::{Browser, BrowserEvent, RequestId};

/// Rows exposed immediately; larger publications continue within idle budgets.
const FIRST_PUBLISH_COUNT: usize = 128;
const STAGE_INLINE_LIMIT: usize = 512;
const PUBLISH_TAIL_CHUNK: usize = 2048;
const PUBLISH_SLICE_BUDGET: Duration = Duration::from_millis(8);

pub(super) enum PublishTerminal {
    LoadFinished {
        truncated: bool,
        retry_metadata: bool,
    },
    SortingFinished,
}

pub(super) struct PublicationPlan {
    pub(super) request_id: RequestId,
    pub(super) total: usize,
    pub(super) focused: Option<usize>,
    pub(super) positions: Vec<usize>,
    pub(super) terminal: PublishTerminal,
}

/// Progress is separate from the captured selection and terminal payload.
pub(super) struct StagedPublish {
    plan: PublicationPlan,
    published: usize,
}

impl StagedPublish {
    fn next_chunk(&self, available: usize) -> (usize, usize) {
        let end = (self.published + PUBLISH_TAIL_CHUNK)
            .min(available)
            .min(self.plan.total);
        (self.published, end.saturating_sub(self.published))
    }
}

impl Browser {
    pub(super) fn publish_staged(self: &Rc<Self>, depth: usize, plan: PublicationPlan) {
        self.drain_publish(depth);
        if plan.total <= STAGE_INLINE_LIMIT {
            if self.state.borrow().columns.get(depth).is_none() {
                return;
            }
            self.emit(BrowserEvent::EntriesReplaced {
                depth,
                count: plan.total,
            });
            self.complete_publication(depth, plan);
            return;
        }
        let published = self
            .state
            .borrow()
            .columns
            .get(depth)
            .map_or(0, |column| column.entries.len().min(FIRST_PUBLISH_COUNT));
        self.emit(BrowserEvent::EntriesReplaced {
            depth,
            count: published,
        });
        self.staged_publishes
            .borrow_mut()
            .insert(depth, StagedPublish { plan, published });
        self.arm_publish_timer();
    }

    fn complete_publication(self: &Rc<Self>, depth: usize, plan: PublicationPlan) {
        if let Some(focused) = plan.focused {
            self.emit(BrowserEvent::SelectionSetChanged {
                depth,
                positions: plan.positions,
                focused,
                take_focus: false,
            });
        }
        match plan.terminal {
            PublishTerminal::LoadFinished {
                truncated,
                retry_metadata,
            } => {
                self.emit(BrowserEvent::LoadFinished { depth, truncated });
                if retry_metadata {
                    self.ensure_sorted_after_load(depth);
                }
            }
            PublishTerminal::SortingFinished => self.emit(BrowserEvent::SortingFinished { depth }),
        }
    }

    /// Converge with the authoritative model before mutations that assume all
    /// rows are visible; unlike an idle tail, draining is not chunk-limited.
    pub(super) fn drain_publish(self: &Rc<Self>, depth: usize) {
        let staged = self.staged_publishes.borrow_mut().remove(&depth);
        let Some(staged) = staged else {
            return;
        };
        let remainder = self.state.borrow().columns.get(depth).map_or(0, |column| {
            column.entries.len().saturating_sub(staged.published)
        });
        if remainder > 0 {
            self.emit(BrowserEvent::EntriesPublished {
                depth,
                position: staged.published,
                count: remainder,
            });
        }
        self.complete_publication(depth, staged.plan);
    }

    pub(super) fn cancel_publish(&self, depth: usize) {
        self.staged_publishes.borrow_mut().remove(&depth);
        if self.staged_publishes.borrow().is_empty()
            && let Some(source) = self.publish_timer.borrow_mut().take()
        {
            source.remove();
        }
    }

    fn arm_publish_timer(self: &Rc<Self>) {
        if self.publish_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        // Run after GDK redraw (priority 120), but before default idle (200):
        // frames stay smooth without letting continuous frame work starve tails.
        let source = glib::idle_add_local_full(glib::Priority::from(130), move || {
            if let Some(browser) = weak.upgrade() {
                browser.fire_publish_tails();
            }
            glib::ControlFlow::Break
        });
        *self.publish_timer.borrow_mut() = Some(source);
    }

    fn fire_publish_tails(self: &Rc<Self>) {
        self.publish_timer.borrow_mut().take();
        let started = Instant::now();
        loop {
            let depth = self.staged_publishes.borrow().keys().copied().next();
            let Some(depth) = depth else {
                return;
            };
            if self.discard_stale_publication(depth) {
                continue;
            }
            if started.elapsed() >= PUBLISH_SLICE_BUDGET {
                self.arm_publish_timer();
                return;
            }
            self.publish_next_chunk(depth);
        }
    }

    fn discard_stale_publication(&self, depth: usize) -> bool {
        let current = self
            .staged_publishes
            .borrow()
            .get(&depth)
            .map(|staged| staged.plan.request_id);
        let stale =
            current.is_some_and(|id| self.state.borrow().request_id_for_depth(depth) != Some(id));
        if stale {
            self.staged_publishes.borrow_mut().remove(&depth);
        }
        stale
    }

    fn next_publication_chunk(&self, depth: usize) -> Option<(usize, usize)> {
        self.staged_publishes
            .borrow()
            .get(&depth)
            .and_then(|staged| {
                self.state
                    .borrow()
                    .columns
                    .get(depth)
                    .map(|column| staged.next_chunk(column.entries.len()))
            })
    }

    fn publish_next_chunk(self: &Rc<Self>, depth: usize) {
        match self.next_publication_chunk(depth) {
            None => {
                self.staged_publishes.borrow_mut().remove(&depth);
            }
            Some((_, 0)) => {
                let staged = self.staged_publishes.borrow_mut().remove(&depth);
                if let Some(staged) = staged {
                    self.complete_publication(depth, staged.plan);
                }
            }
            Some((position, count)) => {
                self.emit(BrowserEvent::EntriesPublished {
                    depth,
                    position,
                    count,
                });
                if let Some(staged) = self.staged_publishes.borrow_mut().get_mut(&depth) {
                    staged.published += count;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
