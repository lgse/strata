// SPDX-License-Identifier: MIT

use crate::model::{FileEntry, Location};
use crate::services::{OperationRequestId, RequestId};
use crate::ui::browser::ViewState;
use gtk::prelude::*;
use std::rc::Rc;

enum PendingRenameState {
    Queued,
    Dispatching,
    Running(OperationRequestId),
    AwaitingRefresh { requests: Vec<(usize, RequestId)> },
}

pub(in crate::ui::browser) struct PendingRename {
    pub(super) old_location: Location,
    pub(super) new_location: Option<Location>,
    old_name: String,
    pub(super) new_name: String,
    /// The item went to a typed path, leaving its row for `new_location`.
    placed: bool,
    generation: u64,
    monitor_reported_rename: bool,
    reveal_generation: u64,
    pub(super) scroll_value: Option<f64>,
    pub(super) source_position: Option<usize>,
    state: PendingRenameState,
}

impl PendingRename {
    fn begin_dispatch(&mut self) -> bool {
        if !matches!(self.state, PendingRenameState::Queued) {
            return false;
        }
        self.state = PendingRenameState::Dispatching;
        true
    }

    fn finish_dispatch(&mut self, operation_id: OperationRequestId) -> bool {
        if !matches!(self.state, PendingRenameState::Dispatching) {
            return false;
        }
        self.state = PendingRenameState::Running(operation_id);
        true
    }

    fn owns_operation(
        &self,
        operation_id: OperationRequestId,
        last_started: Option<OperationRequestId>,
    ) -> bool {
        let dispatched_last = || {
            matches!(&self.state, PendingRenameState::Dispatching)
                && last_started == Some(operation_id)
        };
        matches!(&self.state, PendingRenameState::Running(id) if *id == operation_id)
            || dispatched_last()
    }

    fn complete(
        &mut self,
        operation_id: OperationRequestId,
        last_started: Option<OperationRequestId>,
    ) -> bool {
        if !self.owns_operation(operation_id, last_started) {
            return false;
        }
        self.state = PendingRenameState::AwaitingRefresh {
            requests: Vec::new(),
        };
        true
    }

    /// Where the item's filter result goes: its new path, or `None` when it moves
    /// to another folder.
    fn result_target(&self) -> Option<Option<&Location>> {
        if self.placed {
            // Deliberately not followed, even where a recursive filter lists it again.
            return Some(None);
        }
        self.new_location.as_ref().map(Some)
    }

    fn awaiting_refresh(&self) -> bool {
        matches!(&self.state, PendingRenameState::AwaitingRefresh { .. })
    }

    fn not_yet_running(&self) -> bool {
        matches!(
            self.state,
            PendingRenameState::Queued | PendingRenameState::Dispatching
        )
    }

    fn matches(&self, location: &Location) -> bool {
        let renamed_to = || {
            self.new_location
                .as_ref()
                .is_some_and(|new_location| new_location == location)
        };
        self.old_location == *location || (self.awaiting_refresh() && renamed_to())
    }

    /// Removes the listing request this rename waited for, reporting whether none remain.
    fn finish_refresh(&mut self, depth: usize, request_id: RequestId) -> Option<bool> {
        let PendingRenameState::AwaitingRefresh { requests } = &mut self.state else {
            return None;
        };
        let index = requests
            .iter()
            .position(|(pending_depth, pending_request)| {
                *pending_depth == depth && *pending_request == request_id
            })?;
        requests.remove(index);
        Some(requests.is_empty())
    }
}

/// A directory monitor that started with hidden files off reports a rename to a dot
/// name as the old name's removal, so the new name never reaches the listing.
fn is_dot_name(location: &Location) -> bool {
    location
        .file_name()
        .is_some_and(|name| name.as_encoded_bytes().first() == Some(&b'.'))
}

impl ViewState {
    pub(in crate::ui::browser) fn rename_operation_pending(&self) -> bool {
        self.pending_rename.borrow().is_some()
    }

    pub(in crate::ui) fn pending_rename_name(&self, entry: &FileEntry) -> Option<String> {
        self.pending_rename
            .borrow()
            .as_ref()
            .filter(|pending| pending.matches(&entry.location))
            .map(|pending| pending.new_name.clone())
    }

    fn rename_scroll_value(&self) -> Option<f64> {
        let depth = self.browser.active_depth()?;
        let view = self.rename_collection_view(depth, self.mode_views.borrow().mode())?;
        view.ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .map(|scroll| scroll.vadjustment().value())
    }

    /// `placed` is where the row's replacement appears when the item moves away.
    pub(super) fn start_pending_rename(
        &self,
        entry: &FileEntry,
        new_name: String,
        placed: Option<Option<Location>>,
    ) -> u64 {
        let new_location = placed.clone().unwrap_or_else(|| {
            entry
                .location
                .parent()
                .and_then(|parent| parent.child(std::ffi::OsStr::new(&new_name)))
        });
        let placed = placed.is_some();
        let generation = self.rename_generation.get().saturating_add(1);
        self.rename_generation.set(generation);
        self.pending_rename.replace(Some(PendingRename {
            old_location: entry.location.clone(),
            new_location,
            old_name: entry.display_name.clone(),
            new_name,
            placed,
            generation,
            monitor_reported_rename: false,
            reveal_generation: self.rename_reveal_generation.get(),
            source_position: self.browser.rename_item().map(|(_, position, _)| position),
            scroll_value: self.rename_scroll_value(),
            state: PendingRenameState::Queued,
        }));
        generation
    }

    fn pending_generation_is(&self, generation: u64) -> bool {
        self.pending_rename
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.generation == generation)
    }

    pub(super) fn queue_pending_rename(
        self: &Rc<Self>,
        generation: u64,
        start: impl FnOnce(&Rc<crate::app::Browser>) -> Option<OperationRequestId> + 'static,
    ) {
        let browser = self.browser.clone();
        let operation_at_queue = browser.last_started_operation();
        let weak = Rc::downgrade(self);
        gtk::glib::idle_add_local_once(move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.browser.last_started_operation() != operation_at_queue {
                if state.pending_generation_is(generation) {
                    state.fail_pending_rename();
                }
                return;
            }
            state.dispatch_pending_rename(generation, start);
        });
    }

    fn dispatch_pending_rename(
        &self,
        generation: u64,
        start: impl FnOnce(&Rc<crate::app::Browser>) -> Option<OperationRequestId>,
    ) {
        let dispatch = self
            .pending_rename
            .borrow_mut()
            .as_mut()
            .filter(|pending| pending.generation == generation)
            .is_some_and(PendingRename::begin_dispatch);
        if !dispatch {
            return;
        }
        let Some(operation_id) = start(&self.browser) else {
            return;
        };
        if let Some(pending) = self
            .pending_rename
            .borrow_mut()
            .as_mut()
            .filter(|pending| pending.generation == generation)
        {
            pending.finish_dispatch(operation_id);
        }
    }

    fn rename_parent_is_visible(&self, pending: &PendingRename) -> bool {
        let Some(parent) = pending.old_location.parent() else {
            return false;
        };
        (0..)
            .map_while(|depth| self.browser.column_snapshot(depth))
            .any(|snapshot| snapshot.location == parent)
    }

    pub(in crate::ui::browser) fn note_pending_rename_splices(
        &self,
        depth: usize,
        splices: &[crate::app::EntrySplice],
    ) {
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        let completed = {
            let mut pending = self.pending_rename.borrow_mut();
            let Some(pending) = pending.as_mut() else {
                return;
            };
            if !self.splices_report_rename(pending, (depth, &snapshot), splices) {
                return;
            }
            pending.monitor_reported_rename = true;
            pending.awaiting_refresh()
        };
        if completed {
            self.pending_rename.take();
        }
    }

    /// Whether these splices show the renamed item arriving, or a dot rename's
    /// old row leaving.
    fn splices_report_rename(
        &self,
        pending: &PendingRename,
        (depth, snapshot): (usize, &crate::app::BrowserColumnSnapshot),
        splices: &[crate::app::EntrySplice],
    ) -> bool {
        let Some(new_location) = pending.new_location.as_ref() else {
            return false;
        };
        if pending.old_location.parent().as_ref() != Some(&snapshot.location) {
            return false;
        }
        let mut arrived = splices
            .iter()
            .flat_map(|splice| splice.entries.iter())
            .filter(|entry| &entry.location == new_location)
            .peekable();
        if arrived.peek().is_none() {
            return is_dot_name(new_location)
                && !self.listing_contains(depth, snapshot.count, &pending.old_location);
        }
        for entry in arrived {
            crate::ui::thumbnail::preserve_renamed_thumbnail(&pending.old_location, entry);
        }
        true
    }

    fn listing_contains(&self, depth: usize, count: usize, location: &Location) -> bool {
        self.browser
            .with_entries(depth, 0..count, |entries| {
                entries.iter().any(|entry| &entry.location == location)
            })
            .unwrap_or(false)
    }

    pub(in crate::ui::browser) fn reconcile_pending_rename(&self) {
        let abandoned = self
            .pending_rename
            .borrow()
            .as_ref()
            .is_some_and(|pending| {
                pending.awaiting_refresh() && !self.rename_parent_is_visible(pending)
            });
        if abandoned {
            self.pending_rename.take();
        }
    }

    pub(in crate::ui::browser) fn note_pending_rename_refresh(&self, depth: usize) {
        let Some(request_id) = self.browser.column_request_id(depth) else {
            return;
        };
        let Some(parent) = self
            .pending_rename
            .borrow()
            .as_ref()
            .and_then(|pending| pending.old_location.parent())
        else {
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        if snapshot.location != parent {
            return;
        }
        let mut pending = self.pending_rename.borrow_mut();
        let Some(pending) = pending.as_mut() else {
            return;
        };
        let PendingRenameState::AwaitingRefresh { requests } = &mut pending.state else {
            return;
        };
        requests.retain(|(pending_depth, _)| *pending_depth != depth);
        requests.push((depth, request_id));
    }

    pub(in crate::ui::browser) fn reconcile_pending_rename_after_load(&self, depth: usize) {
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        if snapshot.loading {
            return;
        }
        let Some(request_id) = self.browser.column_request_id(depth) else {
            return;
        };
        let finished = self
            .pending_rename
            .borrow_mut()
            .as_mut()
            .and_then(|pending| pending.finish_refresh(depth, request_id));
        if finished == Some(true) {
            self.pending_rename.take();
        }
    }

    pub(in crate::ui::browser) fn abandon_pending_rename(&self, operation_id: OperationRequestId) {
        let owned = self
            .pending_rename
            .borrow()
            .as_ref()
            .is_some_and(|pending| {
                pending.owns_operation(operation_id, self.browser.last_started_operation())
            });
        if owned {
            self.fail_pending_rename();
        }
    }

    fn pending_rename_reveals(&self) -> bool {
        self.pending_rename
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.reveal_generation == self.rename_reveal_generation.get())
    }

    pub(in crate::ui::browser) fn complete_pending_rename(
        self: &Rc<Self>,
        operation_id: OperationRequestId,
    ) {
        let completed = {
            let mut pending = self.pending_rename.borrow_mut();
            let Some(pending) = pending.as_mut() else {
                return;
            };
            if !pending.complete(operation_id, self.browser.last_started_operation()) {
                return;
            }
            (
                pending.old_location.clone(),
                pending.new_location.clone(),
                pending.new_name.clone(),
                pending.placed,
            )
        };
        let (old_location, new_location, new_name, placed) = completed;
        if placed {
            self.complete_placed_rename(new_location);
            return;
        }
        self.update_rename_labels(&old_location, new_location.as_ref(), &new_name);
        if let Some(location) = new_location
            && self.pending_rename_reveals()
        {
            self.reveal_completed_rename(old_location, location);
        }
        let observed = self
            .pending_rename
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.monitor_reported_rename);
        if observed {
            self.pending_rename.take();
        } else {
            self.reconcile_pending_rename();
        }
    }

    fn complete_placed_rename(self: &Rc<Self>, new_location: Option<Location>) {
        let revealing = self.pending_rename_reveals();
        self.pending_rename.take();
        if revealing && let Some(location) = new_location {
            self.reveal_placed_entry(location);
        }
    }

    pub(in crate::ui::browser) fn complete_typed_placement(
        self: &Rc<Self>,
        operation_id: OperationRequestId,
    ) {
        let placement = self.typed_placement.take();
        if let Some((id, target)) = placement
            && id == operation_id
        {
            self.reveal_placed_entry(target);
        }
    }

    /// The placed item's row leaves its folder, so select what took its place
    /// there, as a rename keeps its row selected.
    fn reveal_placed_entry(self: &Rc<Self>, target: Location) {
        if let Some(parent) = target
            .parent()
            .filter(|parent| self.browser.open_depth(parent).is_some())
        {
            self.apply_transfer_reveal(parent, vec![target]);
        }
    }

    /// Tells filter results where the pending rename takes their item.
    pub(in crate::ui::browser) fn follow_pending_rename(&self) {
        let target = self.pending_rename.borrow().as_ref().and_then(|pending| {
            let to = pending.result_target()?.cloned();
            Some((pending.old_location.clone(), to))
        });
        if let Some((from, to)) = target {
            self.follow_renamed_result(&from, to.as_ref());
        }
    }

    pub(in crate::ui::browser) fn fail_pending_rename(&self) {
        let Some(pending) = self.pending_rename.take() else {
            return;
        };
        if let Some(to) = pending.result_target() {
            self.forget_renamed_result(&pending.old_location, to);
        }
        self.browser.clear_group_folder_for(&pending.old_location);
        self.update_rename_labels(&pending.old_location, None, &pending.old_name);
    }

    pub(in crate::ui::browser) fn fail_pending_rename_from_browser(
        &self,
        operation_id: Option<OperationRequestId>,
    ) {
        let owned =
            self.pending_rename
                .borrow()
                .as_ref()
                .is_some_and(|pending| match operation_id {
                    None => pending.not_yet_running(),
                    Some(actual) => {
                        pending.owns_operation(actual, self.browser.last_started_operation())
                    }
                });
        if owned {
            self.fail_pending_rename();
        }
    }
}
