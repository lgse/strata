// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gio::glib;

use crate::{
    app::navigation::FolderSortResolver,
    model::{FolderSort, Location, SortDirection, SortKey, ViewPreferences},
};

use super::Browser;

/// Columns re-sort one at a time through the debounced sort path, which keeps
/// a single pending sort; each finished sort wakes the next.
#[derive(Default)]
pub(super) struct SortResync {
    pending: Cell<bool>,
    first: Cell<Option<usize>>,
    scheduled: Cell<bool>,
    /// The generation whose debounced apply has not run yet.
    pub(super) debounce: Cell<Option<u64>>,
    /// Sorts started here, which change nothing shared when they finish.
    automatic: Cell<Option<u64>>,
    /// Targets already tried in this pass: a sort abandoned for missing
    /// metadata keeps the column's old order and is not retried.
    attempted: RefCell<Vec<(usize, ViewPreferences)>>,
}

impl Browser {
    /// `Some` remembers sorts per folder: sorting a column then changes only its
    /// folder, and columns follow their folder's saved sort or the default.
    pub fn set_folder_sorts(self: &Rc<Self>, resolver: Option<FolderSortResolver>) {
        self.state.borrow_mut().set_folder_sorts(resolver);
        self.resync_column_sorts();
    }

    pub fn observe_folder_sorts(
        &self,
        observer: impl Fn(&Location, SortKey, SortDirection) + 'static,
    ) {
        self.folder_sort_observers
            .borrow_mut()
            .push(Rc::new(observer));
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

    /// Re-sorts columns whose folder sort, default sort, or folders-first
    /// choice changed elsewhere.
    pub fn resync_column_sorts(self: &Rc<Self>) {
        self.resync_column_sorts_from(None);
    }

    /// Loaded columns re-sort from the main loop, never inside the sort or
    /// preference callback that requested it; nothing is scheduled when no
    /// loaded column changes.
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
        let pending = self.pending_sort.get().map(|(generation, _)| generation);
        (pending.is_some() && self.sort_resync.debounce.get() == pending)
            || self.sort_awaiting_fill.borrow().is_some()
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
            return;
        };
        self.sort_resync
            .attempted
            .borrow_mut()
            .push((depth, target));
        self.apply_column_preferences(depth, move |preferences| {
            preferences.folders_first = target.folders_first;
            preferences.sort_key = target.sort_key;
            preferences.sort_direction = target.sort_direction;
        });
        self.sort_resync
            .automatic
            .set(self.pending_sort.get().map(|(generation, _)| generation));
    }

    /// Columns still loading take their new preferences directly and sort when
    /// the load finishes.
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
            if state.column_preferences(depth) == Some(target)
                || self
                    .sort_resync
                    .attempted
                    .borrow()
                    .contains(&(depth, target))
            {
                continue;
            }
            if state.columns[depth].load_state == crate::app::navigation::LoadState::Loading {
                state.set_loading_column_preferences(depth, target);
                continue;
            }
            return Some((depth, target));
        }
        None
    }

    /// Without per-folder sorting an explicit sort becomes the default for new
    /// columns, as before; with it, only the column's folder is reported.
    pub(super) fn record_sort(&self, depth: usize, generation: u64) {
        if self.sort_resync.automatic.get() == Some(generation) {
            return;
        }
        let (location, sorted, remembered) = {
            let mut state = self.state.borrow_mut();
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
                if sorted.sort_key != SortKey::DeviceOrder {
                    let mut defaults = self.preferences.get();
                    defaults.sort_key = sorted.sort_key;
                    defaults.sort_direction = sorted.sort_direction;
                    self.preferences.set(defaults);
                    state.set_default_preferences(defaults);
                }
                drop(state);
                self.notify_preferences_observers();
                return;
            }
            let remembered = state.folder_sort(&location) != FolderSort::Unremembered;
            (location, sorted, remembered)
        };
        if !remembered || matches!(sorted.sort_key, SortKey::DeviceOrder | SortKey::Recency) {
            return;
        }
        let observers = self.folder_sort_observers.borrow().clone();
        for observer in &observers {
            observer(&location, sorted.sort_key, sorted.sort_direction);
        }
    }
}
