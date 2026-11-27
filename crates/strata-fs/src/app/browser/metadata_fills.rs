// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet, VecDeque},
    rc::{Rc, Weak},
};

use crate::{
    app::navigation::{EntrySplice, NavigationState},
    model::{Location, SortKey, ViewPreferences},
    services::{MetadataOutcome, MetadataRequest, RequestId},
};

use super::{
    Browser, DIRECTORY_LOAD_TIME_BUDGET, MAX_PENDING_FILL_LOCATIONS, MAX_VIEWPORT_FILL_BATCH,
    METADATA_FILL_TIME_BUDGET, MediaSortFill, PublicationPlan, PublishTerminal, SortFill,
    SortOrigin, ViewportFill, ViewportTarget,
};

const MEDIA_SORT_BATCH_SIZE: usize = 256;

pub(super) struct SortFillBatch {
    locations: Vec<Location>,
    remaining: usize,
}

impl SortFillBatch {
    pub(super) fn new(state: &NavigationState, depth: usize, key: SortKey) -> Self {
        let mut targets = state.column_unknown_sort_metadata(depth, key);
        let limit = if key.uses_media_metadata() {
            MEDIA_SORT_BATCH_SIZE
        } else {
            usize::MAX
        };
        let locations: Vec<_> = targets
            .by_ref()
            .take(limit)
            .map(|(_, location)| location.clone())
            .collect();
        Self {
            remaining: locations.len() + targets.count(),
            locations,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }
}

impl Browser {
    pub fn request_metadata_fill(
        self: &Rc<Self>,
        depth: usize,
        position: usize,
        location: Location,
        include_icon_details: bool,
    ) {
        let target = ViewportTarget::new(position, location, include_icon_details);
        self.queue_metadata_fill(depth, target, false);
    }

    pub fn request_visible_metadata_fill(
        self: &Rc<Self>,
        depth: usize,
        position: usize,
        location: Location,
        include_icon_details: bool,
    ) {
        let target = ViewportTarget::new(position, location, include_icon_details);
        self.queue_metadata_fill(depth, target, true);
    }

    fn queue_metadata_fill(self: &Rc<Self>, depth: usize, target: ViewportTarget, visible: bool) {
        // Defer to the provider instead of rejecting remote locations owner-side:
        // unsupported sources answer `Unsupported`.
        if !self.source.supports_metadata_fill(&target.location) {
            return;
        }
        if self.fill_in_flight(depth, &target) {
            return;
        }
        self.metadata_pending
            .borrow_mut()
            .entry(depth)
            .or_default()
            .enqueue(target, visible);
        self.schedule_metadata_fill();
    }

    fn fill_in_flight(&self, depth: usize, target: &ViewportTarget) -> bool {
        let directory_request = self.state.borrow().request_id_for_depth(depth);
        self.fill_tokens
            .borrow()
            .values()
            .any(|fill| fill.covers(depth, directory_request, target))
    }

    pub(super) fn invalidate_spliced_viewport_details(
        &self,
        depth: usize,
        splices: &[EntrySplice],
    ) {
        let mut fills = self.fill_tokens.borrow_mut();
        for fill in fills
            .values_mut()
            .filter(|fill| fill.depth == depth && fill.include_icon_details)
        {
            fill.tokens.retain(|location, _| {
                !splices
                    .iter()
                    .flat_map(|splice| &splice.entries)
                    .any(|entry| !entry.is_directory() && entry.location == *location)
            });
        }
    }

    pub fn prioritize_metadata_fills(&self, depth: usize, visible: &[Location]) {
        let mut pending = self.metadata_pending.borrow_mut();
        let Some(targets) = pending.get_mut(&depth) else {
            return;
        };
        targets.prioritize(visible);
    }

    pub(super) fn request_sort_fill(self: &Rc<Self>, pass: SortPass, batch: SortFillBatch) {
        let SortPass {
            depth,
            generation,
            origin,
            preferences,
        } = pass;
        let Some(directory_request) = self.state.borrow().request_id_for_depth(depth) else {
            self.sort_resync.note_attempt(origin, depth, preferences);
            self.pending_sort.set(None);
            self.finish_sorting(depth);
            return;
        };
        let media = preferences
            .sort_key
            .uses_media_metadata()
            .then(|| MediaSortFill {
                revision: self.sort_resync.media_revision(depth, directory_request),
                remaining_before: batch.remaining,
            });
        let fill_request = self.new_request_id();
        self.sort_awaiting_fill.borrow_mut().replace(SortFill {
            generation,
            depth,
            origin,
            fill_request,
            directory_request,
            preferences,
            media,
        });
        let weak: Weak<Self> = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_event(event);
            }
        });
        let handle = self.source.fill_metadata(
            MetadataRequest {
                id: fill_request,
                entries: batch.locations,
                full: true,
                include_icon_details: media.is_some(),
                time_budget: DIRECTORY_LOAD_TIME_BUDGET,
            },
            emit,
        );
        if self
            .sort_awaiting_fill
            .borrow()
            .as_ref()
            .is_some_and(|fill| fill.fill_request == fill_request)
        {
            self.sort_loads.borrow_mut().insert(depth, handle);
        }
    }

    pub(super) fn finish_awaited_sort(self: &Rc<Self>, awaiting: SortFill) {
        let SortFill {
            depth,
            generation,
            origin,
            mut preferences,
            ..
        } = awaiting;
        // Metadata can arrive after another window changes the application-wide visibility.
        preferences.show_hidden = self.preferences.get().show_hidden;
        self.sort_awaiting_fill.borrow_mut().take();
        self.sort_loads.borrow_mut().remove(&depth);
        self.state.borrow_mut().clear_metadata_positions(depth);
        let outcome = {
            let mut state = self.state.borrow_mut();
            if self.pending_sort.get() != Some((generation, depth)) {
                return;
            }
            self.sort_resync.note_attempt(origin, depth, preferences);
            let outcome = state.apply_sort_preferences(depth, preferences);
            self.pending_sort_applied.set(true);
            let take_focus = state.take_selection_from_reveal(depth);
            outcome.and_then(|(focused, positions)| {
                Some(PublicationPlan {
                    request_id: state.request_id_for_depth(depth)?,
                    total: state.columns.get(depth)?.entries.len(),
                    focused,
                    positions,
                    take_focus,
                    terminal: PublishTerminal::SortingFinished,
                })
            })
        };
        if origin == SortOrigin::Chosen {
            self.record_sort(depth);
        }
        if let Some(plan) = outcome {
            self.publish_staged(depth, plan);
        } else {
            self.pending_sort.set(None);
            self.finish_sorting(depth);
        }
    }

    pub(super) fn handle_metadata_finished(
        self: &Rc<Self>,
        request_id: RequestId,
        outcome: MetadataOutcome,
    ) {
        let awaiting = *self.sort_awaiting_fill.borrow();
        if let Some(awaiting) = awaiting
            && awaiting.fill_request == request_id
        {
            self.finish_sort_fill(awaiting, outcome);
            return;
        }
        // Only a fill's own id releases its handle; terminals from superseded fills
        // cannot affect a sort or a newer request.
        let fill = self.fill_tokens.borrow_mut().remove(&request_id);
        if let Some(fill) = fill {
            self.release_viewport_fill(fill.depth);
        }
    }

    fn finish_sort_fill(self: &Rc<Self>, awaiting: SortFill, outcome: MetadataOutcome) {
        self.sort_loads.borrow_mut().remove(&awaiting.depth);
        let current = self.pending_sort.get() == Some((awaiting.generation, awaiting.depth));
        if current && let Some(media) = awaiting.media {
            self.sort_resync
                .finish_media_revision(awaiting.depth, media.revision);
            if matches!(
                outcome,
                MetadataOutcome::Complete | MetadataOutcome::Truncated
            ) {
                self.continue_media_sort(awaiting, media.remaining_before);
                self.finish_awaited_sort(awaiting);
                return;
            }
        }
        if current && outcome == MetadataOutcome::Complete {
            self.finish_awaited_sort(awaiting);
            return;
        }
        if current {
            self.sort_resync
                .note_attempt(awaiting.origin, awaiting.depth, awaiting.preferences);
        }
        self.abandon_awaited_sort(awaiting.depth, awaiting.generation, outcome);
    }

    fn continue_media_sort(self: &Rc<Self>, fill: SortFill, remaining_before: usize) {
        let remaining = self
            .state
            .borrow()
            .column_unknown_sort_metadata(fill.depth, fill.preferences.sort_key)
            .count();
        if remaining > 0 && remaining < remaining_before {
            self.schedule_media_sort(fill.depth);
        }
    }

    fn release_viewport_fill(self: &Rc<Self>, depth: usize) {
        self.metadata_loads.borrow_mut().remove(&depth);
        if self.metadata_pending.borrow().contains_key(&depth) {
            self.schedule_metadata_fill();
        }
    }

    /// Every `SortingStarted` still pairs with exactly one `SortingFinished`.
    pub(super) fn abandon_awaited_sort(
        &self,
        depth: usize,
        generation: u64,
        outcome: MetadataOutcome,
    ) {
        self.sort_awaiting_fill.borrow_mut().take();
        self.sort_loads.borrow_mut().remove(&depth);
        self.state.borrow_mut().clear_metadata_positions(depth);
        if self.pending_sort.get() != Some((generation, depth)) {
            return;
        }
        self.pending_sort.set(None);
        tracing::warn!(
            depth,
            generation,
            ?outcome,
            "metadata sort abandoned; prior order preserved"
        );
        self.finish_sorting(depth);
    }

    pub(super) fn cancel_pending_sort_for(&self, depth: usize) {
        let awaiting = *self.sort_awaiting_fill.borrow();
        if let Some(awaiting) = awaiting
            && awaiting.depth == depth
        {
            self.abandon_awaited_sort(depth, awaiting.generation, MetadataOutcome::Cancelled);
            return;
        }
        self.sort_loads.borrow_mut().remove(&depth);
        // An applied sort is finished by its publication, or by cancelling it.
        if !self.pending_sort_applied.get()
            && self
                .pending_sort
                .get()
                .is_some_and(|(_, pending_depth)| pending_depth == depth)
        {
            self.pending_sort.set(None);
            self.finish_sorting(depth);
        }
    }

    pub(super) fn ensure_sorted_after_load(self: &Rc<Self>, depth: usize) {
        let (needs, preferences) = {
            let state = self.state.borrow();
            let Some(preferences) = state.column_preferences(depth) else {
                return;
            };
            let needs = state
                .column_unknown_sort_metadata(depth, preferences.sort_key)
                .next()
                .is_some();
            (needs, preferences)
        };
        if !needs {
            return;
        }
        if preferences.sort_key.uses_media_metadata() {
            self.schedule_media_sort(depth);
            return;
        }
        let generation = self.begin_sort(depth);
        let batch = SortFillBatch::new(&self.state.borrow(), depth, preferences.sort_key);
        let pass = SortPass {
            depth,
            generation,
            origin: SortOrigin::Followed,
            preferences,
        };
        self.request_sort_fill(pass, batch);
    }

    pub(super) fn flush_metadata_fills(self: &Rc<Self>) {
        self.metadata_idle.borrow_mut().take();
        // Newly bound rows must not cancel late details for cards that remain bound.
        let active_depths: HashSet<usize> = self
            .fill_tokens
            .borrow()
            .values()
            .map(|fill| fill.depth)
            .collect();
        let pending: Vec<(usize, PendingQueue)> = self
            .metadata_pending
            .borrow_mut()
            .extract_if(|depth, _| !active_depths.contains(depth))
            .collect();
        for (depth, mut queue) in pending {
            let Some(directory_request) = self.state.borrow().request_id_for_depth(depth) else {
                continue;
            };
            // Bound the non-preemptible batch so scrolling can reprioritize the backlog,
            // and keep icon-details targets separate from basic-only targets.
            let Some((targets, include_icon_details)) = queue.take_batch(MAX_VIEWPORT_FILL_BATCH)
            else {
                continue;
            };
            if !queue.is_empty() {
                self.metadata_pending.borrow_mut().insert(depth, queue);
            }
            let fill_request = self.new_request_id();
            let weak: Weak<Self> = Rc::downgrade(self);
            let emit = Rc::new(move |event| {
                if let Some(browser) = weak.upgrade() {
                    browser.handle_directory_event(event);
                }
            });
            let tokens: HashMap<Location, usize> = targets
                .iter()
                .map(|target| (target.location.clone(), target.position))
                .collect();
            // Stored before the provider runs: synchronous fills answer inside the call.
            self.fill_tokens.borrow_mut().insert(
                fill_request,
                ViewportFill {
                    depth,
                    directory_request,
                    tokens,
                    include_icon_details,
                },
            );
            let handle = self.source.fill_metadata(
                MetadataRequest {
                    id: fill_request,
                    entries: targets.into_iter().map(|target| target.location).collect(),
                    full: false,
                    include_icon_details,
                    time_budget: METADATA_FILL_TIME_BUDGET,
                },
                emit,
            );
            if self.fill_tokens.borrow().contains_key(&fill_request) {
                self.metadata_loads.borrow_mut().insert(depth, handle);
            }
        }
    }
}

/// One column sort waiting for the metadata it orders by.
#[derive(Clone, Copy)]
pub(super) struct SortPass {
    pub(super) depth: usize,
    pub(super) generation: u64,
    pub(super) origin: SortOrigin,
    pub(super) preferences: ViewPreferences,
}

impl ViewportTarget {
    fn new(position: usize, location: Location, include_icon_details: bool) -> Self {
        Self {
            position,
            location,
            include_icon_details,
        }
    }
}

impl ViewportFill {
    /// Whether this fill already asks for `target` under the column's current listing.
    fn covers(
        &self,
        depth: usize,
        directory_request: Option<RequestId>,
        target: &ViewportTarget,
    ) -> bool {
        self.depth == depth
            && Some(self.directory_request) == directory_request
            && (!target.include_icon_details || self.include_icon_details)
            && self.tokens.get(&target.location) == Some(&target.position)
    }
}

/// O(1)-amortized bounded priority queue for pending viewport metadata fills.
///
/// Promotions push a newer sequence number to the front instead of scanning the
/// deque, and pops ignore older sequence numbers so back-eviction never drops a
/// promoted entry.
#[derive(Default)]
pub(super) struct PendingQueue {
    order: VecDeque<(Location, u64)>,
    entries: HashMap<Location, (ViewportTarget, u64)>,
    next_seq: u64,
}

impl PendingQueue {
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    fn alloc_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }

    pub(super) fn enqueue(&mut self, target: ViewportTarget, visible: bool) {
        if self.entries.contains_key(&target.location) {
            let promoted_seq = visible.then(|| self.alloc_seq());
            if let Some((existing, seq)) = self.entries.get_mut(&target.location) {
                existing.position = target.position;
                existing.include_icon_details |= target.include_icon_details;
                if let Some(new_seq) = promoted_seq {
                    *seq = new_seq;
                    self.order.push_front((target.location, new_seq));
                    self.compact_if_bloated();
                }
            }
            return;
        }
        if visible {
            // A saturated offscreen backlog must not strand newly visible details.
            if self.entries.len() == MAX_PENDING_FILL_LOCATIONS {
                self.evict_back();
            }
            let seq = self.alloc_seq();
            let loc = target.location.clone();
            self.entries.insert(loc.clone(), (target, seq));
            self.order.push_front((loc, seq));
            self.compact_if_bloated();
        } else if self.entries.len() < MAX_PENDING_FILL_LOCATIONS {
            let seq = self.alloc_seq();
            let loc = target.location.clone();
            self.entries.insert(loc.clone(), (target, seq));
            self.order.push_back((loc, seq));
        }
    }

    pub(super) fn prioritize(&mut self, visible: &[Location]) {
        if self.entries.is_empty() || visible.is_empty() {
            return;
        }
        let mut moved = HashSet::with_capacity(visible.len());
        let mut next = VecDeque::with_capacity(self.entries.len());
        for loc in visible {
            if let Some((_, seq)) = self.entries.get(loc)
                && moved.insert(loc.clone())
            {
                next.push_back((loc.clone(), *seq));
            }
        }
        if next.is_empty() {
            return;
        }
        for (loc, seq) in self.order.drain(..) {
            if !moved.contains(&loc)
                && self
                    .entries
                    .get(&loc)
                    .is_some_and(|(_, current)| *current == seq)
            {
                next.push_back((loc, seq));
            }
        }
        self.order = next;
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&ViewportTarget) -> bool) {
        self.entries.retain(|_, (target, _)| keep(target));
        self.order.retain(|(loc, seq)| {
            self.entries
                .get(loc)
                .is_some_and(|(_, current)| *current == *seq)
        });
    }

    pub(super) fn take_batch(&mut self, limit: usize) -> Option<(Vec<ViewportTarget>, bool)> {
        let mut batch = Vec::with_capacity(limit.min(self.entries.len()));
        let mut skipped = Vec::new();
        let mut batch_details = None;
        while batch.len() < limit {
            let Some((loc, seq)) = self.order.pop_front() else {
                break;
            };
            let Some((target, current)) = self.entries.get(&loc) else {
                continue;
            };
            if *current != seq {
                continue;
            }
            let want_details = *batch_details.get_or_insert(target.include_icon_details);
            if target.include_icon_details == want_details {
                if let Some((target, _)) = self.entries.remove(&loc) {
                    batch.push(target);
                }
            } else {
                skipped.push((loc, seq));
            }
        }
        for item in skipped.into_iter().rev() {
            self.order.push_front(item);
        }
        batch_details.map(|details| (batch, details))
    }

    fn evict_back(&mut self) {
        while let Some((loc, seq)) = self.order.pop_back() {
            let is_live = self
                .entries
                .get(&loc)
                .is_some_and(|(_, current)| *current == seq);
            if is_live {
                self.entries.remove(&loc);
                return;
            }
        }
    }

    fn compact_if_bloated(&mut self) {
        if self.order.len() <= MAX_PENDING_FILL_LOCATIONS * 2 {
            return;
        }
        self.order.retain(|(loc, seq)| {
            self.entries
                .get(loc)
                .is_some_and(|(_, current)| *current == *seq)
        });
    }
}

#[cfg(test)]
impl From<Vec<ViewportTarget>> for PendingQueue {
    fn from(targets: Vec<ViewportTarget>) -> Self {
        let mut queue = Self::default();
        for target in targets {
            queue.enqueue(target, false);
        }
        queue
    }
}
