// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gio::glib;

use crate::{
    app::navigation::{EntrySplice, FolderSortResolver, sort_metadata_missing},
    model::{FolderSort, SortKey, ViewPreferences},
    services::RequestId,
};

use super::{Browser, BrowserEvent};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SortOrigin {
    Chosen,
    /// Follows a saved sort, the default, or a column load; saves nothing.
    Followed,
}

#[derive(Default)]
pub(super) struct SortResync {
    pending: Cell<bool>,
    first: Cell<Option<usize>>,
    scheduled: Cell<bool>,
    attempted: RefCell<Vec<(usize, ViewPreferences)>>,
    media_changes: RefCell<HashMap<usize, (RequestId, RequestId)>>,
}

impl SortResync {
    pub(super) fn media_revision(&self, depth: usize, directory: RequestId) -> Option<RequestId> {
        self.media_changes
            .borrow()
            .get(&depth)
            .and_then(|&(request, revision)| (request == directory).then_some(revision))
    }

    pub(super) fn finish_media_revision(&self, depth: usize, revision: Option<RequestId>) {
        if let Some(revision) = revision {
            let mut changes = self.media_changes.borrow_mut();
            if changes
                .get(&depth)
                .is_some_and(|&(_, pending)| pending == revision)
            {
                changes.remove(&depth);
            }
        }
    }

    pub(super) fn note_attempt(
        &self,
        origin: SortOrigin,
        depth: usize,
        preferences: ViewPreferences,
    ) {
        if origin == SortOrigin::Followed && self.pending.get() {
            self.attempted.borrow_mut().push((depth, preferences));
        }
    }
}

impl Browser {
    pub fn set_folder_sorts(self: &Rc<Self>, resolver: Option<FolderSortResolver>) {
        self.state.borrow_mut().set_folder_sorts(resolver);
        self.resync_column_sorts();
    }

    pub fn folder_sort_at(&self, depth: usize) -> FolderSort {
        let state = self.state.borrow();
        state
            .columns
            .get(depth)
            .map_or(FolderSort::Unremembered, |column| {
                state.folder_sort(&column.location)
            })
    }

    pub fn resync_column_sorts(self: &Rc<Self>) {
        self.resync_column_sorts_from(None);
    }

    pub(super) fn note_media_sort_changes(self: &Rc<Self>, depth: usize, splices: &[EntrySplice]) {
        let Some(key) = self
            .requested_column_preferences(depth)
            .map(|prefs| prefs.sort_key)
        else {
            return;
        };
        if !key.uses_media_metadata()
            || !splices
                .iter()
                .flat_map(|splice| &splice.entries)
                .any(|entry| sort_metadata_missing(key, entry))
        {
            return;
        }
        self.schedule_media_sort(depth);
    }

    pub(super) fn schedule_media_sort(self: &Rc<Self>, depth: usize) {
        let request = self.state.borrow().request_id_for_depth(depth);
        if let Some(request) = request {
            self.sort_resync
                .media_changes
                .borrow_mut()
                .insert(depth, (request, self.new_request_id()));
            self.sort_resync.pending.set(true);
            if !self.sort_in_flight() {
                self.schedule_sort_resync();
            }
        }
    }

    pub(super) fn resync_column_sorts_from(self: &Rc<Self>, first: Option<usize>) {
        if first.is_some() {
            self.sort_resync.first.set(first);
        }
        self.sort_resync.attempted.borrow_mut().clear();
        if self.next_resync_column().is_none() {
            return;
        }
        self.sort_resync.pending.set(true);
        if !self.sort_in_flight() {
            self.schedule_sort_resync();
        }
    }

    pub(super) fn wake_sort_resync(&self) {
        if self.sort_resync.pending.get()
            && let Some(browser) = self.self_weak.upgrade()
        {
            browser.schedule_sort_resync();
        }
    }

    #[cfg(test)]
    pub(super) fn sort_resync_settled(&self) -> bool {
        !self.sort_resync.pending.get() && !self.sort_resync.scheduled.get()
    }

    fn schedule_sort_resync(self: &Rc<Self>) {
        if self.sort_resync.scheduled.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(browser) = weak.upgrade() {
                browser.sort_resync.scheduled.set(false);
                browser.drain_sort_resync();
            }
        });
    }

    fn sort_in_flight(&self) -> bool {
        self.pending_sort.get().is_some_and(|(_, depth)| {
            !self.pending_sort_applied.get() || self.publishing_sort(depth)
        })
    }

    /// A sort still in flight wakes this again when it finishes.
    fn drain_sort_resync(self: &Rc<Self>) {
        if !self.sort_resync.pending.get() || self.sort_in_flight() {
            return;
        }
        let Some((depth, target)) = self.next_resync_column() else {
            self.sort_resync.pending.set(false);
            self.sort_resync.first.set(None);
            self.sort_resync.attempted.borrow_mut().clear();
            self.sort_resync.media_changes.borrow_mut().clear();
            return;
        };
        self.apply_column_preferences(depth, SortOrigin::Followed, move |preferences| {
            preferences.folders_first = target.folders_first;
            preferences.set_sort_options(target.sort_options());
        });
    }

    fn next_resync_column(&self) -> Option<(usize, ViewPreferences)> {
        let mut state = self.state.borrow_mut();
        let first = self
            .sort_resync
            .first
            .get()
            .filter(|depth| *depth < state.columns.len());
        let depths = first
            .into_iter()
            .chain((0..state.columns.len()).filter(|depth| Some(*depth) != first));
        for depth in depths.collect::<Vec<_>>() {
            let Some(target) = state.synchronized_preferences(depth) else {
                continue;
            };
            let media_changed = target.sort_key.uses_media_metadata()
                && state.request_id_for_depth(depth).is_some_and(|request| {
                    self.sort_resync.media_revision(depth, request).is_some()
                });
            if state.column_preferences(depth) == Some(target) && !media_changed {
                continue;
            }
            if state.set_loading_column_preferences(depth, target) {
                continue;
            }
            if !media_changed
                && self
                    .sort_resync
                    .attempted
                    .borrow()
                    .contains(&(depth, target))
            {
                continue;
            }
            return Some((depth, target));
        }
        None
    }

    pub(super) fn record_sort(&self, depth: usize) {
        let (location, sorted, remembered) = {
            let state = self.state.borrow();
            let Some(column) = state.columns.get(depth) else {
                return;
            };
            if column.location.is_recent_root() {
                return;
            }
            let location = column.location.clone();
            let Some(sorted) = state.column_preferences(depth) else {
                return;
            };
            if !state.remembers_folder_sorts() {
                drop(state);
                self.update_defaults(|defaults| {
                    if sorted.sort_key != SortKey::DeviceOrder {
                        defaults.set_sort_options(sorted.sort_options());
                    }
                });
                return;
            }
            let remembered = state.folder_sort(&location) != FolderSort::Unremembered;
            (location, sorted, remembered)
        };
        if !remembered || matches!(sorted.sort_key, SortKey::DeviceOrder | SortKey::Recency) {
            return;
        }
        self.emit(BrowserEvent::FolderSortChosen {
            location,
            sort_key: sorted.sort_key,
            sort_direction: sorted.sort_direction,
            name_comparison: sorted.name_comparison,
        });
    }
}
