// SPDX-License-Identifier: MIT

use std::rc::{Rc, Weak};

use crate::{
    app::navigation::{LoadState, NavigationPath},
    model::Location,
    services::{DirectoryRequest, LocationValidationError},
};

use super::{
    Browser, BrowserEvent, LoadSelection, PEEK_MAX_ENTRIES, PEEK_TIME_BUDGET, SelectionUpdate,
    select_on_load,
};

impl Browser {
    pub fn descend(self: &Rc<Self>, parent_depth: usize, location: Location) {
        self.descend_with_selection(Descent::new(parent_depth, location, LoadSelection::Nothing));
    }

    pub fn show_child(self: &Rc<Self>, parent_depth: usize, location: Location) {
        let descent = Descent::new(parent_depth, location, LoadSelection::Nothing);
        self.descend_with_selection(descent.keeping_parent_active());
    }

    pub(super) fn descend_with_selection(self: &Rc<Self>, descent: Descent) {
        let _update = self.navigation_update();
        self.cancel_deferred_reveal();
        self.bump_navigation_generation();
        if self.is_open_child(descent.parent_depth, &descent.location) {
            return;
        }
        self.close_peek();
        if descent.keep_parent_active {
            self.descend_validated(descent);
            return;
        }
        self.validate_descent(descent);
    }

    fn validate_descent(self: &Rc<Self>, descent: Descent) {
        let generation = self.bump_navigation_generation();
        if self.navigation_generation() != generation {
            return;
        }
        self.navigation_lifecycle.pending.set(Some(generation));
        let weak = Rc::downgrade(self);
        let location = descent.location.clone();
        let pending = PendingDescent {
            generation,
            parent_location: self.location_at(descent.parent_depth),
            descent,
        };
        let emit = Rc::new(move |result| {
            if let Some(browser) = weak.upgrade() {
                browser.finish_descent_validation(&pending, result);
            }
        });
        let load = self.source.validate_location_async(location, emit);
        if self.pending_navigation_generation() == Some(generation) {
            self.validation_load.replace(Some(load));
        }
    }

    fn finish_descent_validation(
        self: &Rc<Self>,
        pending: &PendingDescent,
        result: Result<(), LocationValidationError>,
    ) {
        let _update = self.navigation_update();
        if !self.finish_navigation_validation(pending.generation) {
            return;
        }
        let descent = &pending.descent;
        if self.location_at(descent.parent_depth) != pending.parent_location {
            return;
        }
        match result {
            Err(error) => {
                self.emit(BrowserEvent::NavigationRejected {
                    parent_depth: descent.parent_depth,
                    location: descent.location.clone(),
                    error,
                });
                self.focus_active();
            }
            Ok(()) => self.descend_validated(descent.clone()),
        }
    }

    pub(super) fn descend_validated(self: &Rc<Self>, descent: Descent) {
        let Descent {
            parent_depth,
            location,
            selection,
            keep_parent_active,
        } = descent;
        if !self.source.allows_navigation(&location) || self.location_at(parent_depth).is_none() {
            return;
        }
        self.emit(BrowserEvent::NavigationStarting { history: false });
        let request_id = self.new_request_id();
        let mut state = self.state.borrow_mut();
        if !state.descend(parent_depth, location.clone(), request_id) {
            return;
        }
        if keep_parent_active {
            state.focus_column(parent_depth);
        }
        select_on_load(&mut state, parent_depth + 1, selection);
        drop(state);

        let retained = parent_depth + 1;
        self.loads.borrow_mut().truncate(retained);
        self.monitors.borrow_mut().truncate(retained);
        self.truncate_deferred_from(retained);
        self.emit(BrowserEvent::ColumnsTruncated {
            len: retained,
            replacing: true,
        });
        self.emit(BrowserEvent::ColumnAdded {
            depth: retained,
            location: location.clone(),
        });
        self.emit(BrowserEvent::FocusChanged {
            depth: retained,
            position: None,
            triggered_by_removal: false,
        });
        self.start_load(retained, location, request_id);
    }

    pub fn begin_peek(self: &Rc<Self>, origin_depth: usize, location: Location) {
        self.close_peek();
        if self.is_open_child(origin_depth, &location) {
            return;
        }
        let request_id = self.new_request_id();
        if !self
            .state
            .borrow_mut()
            .begin_peek(origin_depth, location.clone(), request_id)
        {
            return;
        }

        self.emit(BrowserEvent::PeekStarted {
            location: location.clone(),
        });
        let weak: Weak<Self> = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_event(event);
            }
        });
        // Peeks stay small and show metadata immediately, so they skip the streaming split.
        let handle = self.source.enumerate(
            DirectoryRequest {
                id: request_id,
                location,
                batch_size: 128,
                include_metadata: true,
                max_entries: PEEK_MAX_ENTRIES,
                time_budget: PEEK_TIME_BUDGET,
                background: false,
            },
            emit,
        );
        self.peek_load.replace(Some(handle));
    }

    pub fn close_peek(&self) -> bool {
        self.peek_load.take();
        let closed = self.state.borrow_mut().clear_peek();
        if closed {
            self.emit(BrowserEvent::PeekClosed);
        }
        closed
    }

    pub fn clear_active_selection(&self) -> bool {
        let cleared = self.state.borrow_mut().clear_active_selection();
        if let Some((depth, focused)) = cleared {
            self.emit(BrowserEvent::SelectionSetChanged {
                depth,
                selection: SelectionUpdate::Positions(Vec::new()),
                focused,
                take_focus: false,
            });
            return true;
        }
        false
    }

    pub fn escape(self: &Rc<Self>) {
        let _update = self.navigation_update();
        if self.close_peek() || self.clear_active_selection() {
            return;
        }

        let closed = self.state.borrow_mut().close_deepest();
        if let Some((depth, position)) = closed {
            if self.pending_navigation_generation().is_some() {
                self.bump_navigation_generation();
            }
            let len = depth + 1;
            self.loads.borrow_mut().truncate(len);
            self.monitors.borrow_mut().truncate(len);
            self.truncate_deferred_from(len);
            self.emit(BrowserEvent::ColumnsTruncated {
                len,
                replacing: false,
            });
            self.emit_suppressed_focus(depth, position);
        }
    }

    pub fn close_column(self: &Rc<Self>, depth: usize) {
        self.truncate_columns(depth, true);
    }

    /// Columns shown again after another view: the active column's child column
    /// follows its selection. Closes a child the selection no longer holds and
    /// returns the lone selected folder that should open in its place.
    pub fn follow_selection_on_return(self: &Rc<Self>) -> Option<(usize, Location)> {
        // Closing or opening a column would drop a navigation still validating.
        if self.pending_navigation_generation().is_some() {
            return None;
        }
        let depth = self.active_depth()?;
        let (open_child, selected, load_cursor) = {
            let state = self.state.borrow();
            // A loading column has not restored its selection yet.
            if state.columns.get(depth)?.load_state == LoadState::Loading {
                return None;
            }
            let selected: Vec<_> = state
                .selected_positions(depth)
                .into_iter()
                .filter_map(|position| state.entry_at(depth, position))
                .collect();
            (
                state.location_at(depth + 1),
                selected,
                state.selection_is_load_cursor(),
            )
        };
        if open_child
            .as_ref()
            .is_some_and(|child| selected.iter().any(|entry| &entry.location == child))
        {
            return None;
        }
        if open_child.is_some() {
            self.truncate_columns(depth + 1, false);
        }
        // A cursor a load placed on its own is not a choice to open that folder.
        match selected.as_slice() {
            [entry] if entry.is_directory() && !load_cursor => {
                Some((depth, entry.location.clone()))
            }
            _ => None,
        }
    }

    pub(super) fn truncate_columns(self: &Rc<Self>, depth: usize, record_history: bool) {
        let _update = self.navigation_update();
        self.close_peek();
        let closed = {
            let mut state = self.state.borrow_mut();
            if record_history {
                state.close_from(depth)
            } else {
                state.drop_columns_from(depth)
            }
        };
        if let Some((parent_depth, position)) = closed {
            if self.pending_navigation_generation().is_some() {
                self.bump_navigation_generation();
            }
            self.loads.borrow_mut().truncate(depth);
            self.monitors.borrow_mut().truncate(depth);
            self.truncate_deferred_from(depth);
            self.emit(BrowserEvent::ColumnsTruncated {
                len: depth,
                replacing: false,
            });
            self.emit_suppressed_focus(parent_depth, position);
        }
    }

    pub(super) fn emit_suppressed_focus(&self, depth: usize, position: Option<usize>) {
        let was = self.suppress_child_mirror.replace(true);
        self.emit(BrowserEvent::FocusChanged {
            depth,
            position,
            triggered_by_removal: false,
        });
        self.suppress_child_mirror.set(was);
    }

    pub(crate) fn child_mirror_suppressed(&self) -> bool {
        self.suppress_child_mirror.get()
    }

    /// Whether the `FocusChanged` being observed comes from an outside change, not a cursor move.
    pub(crate) fn focus_follows_external_change(&self) -> bool {
        self.external_change_focus.get()
    }

    /// Whether the `FocusChanged` being observed only moves a finished load's cursor,
    /// so it must not pull focus from elsewhere in the window.
    pub(crate) fn focus_follows_background_load(&self) -> bool {
        self.background_load_focus.get()
    }

    pub fn commit_peek(self: &Rc<Self>) {
        let target = self.state.borrow().peek_target();
        if let Some((origin_depth, location)) = target {
            self.close_peek();
            self.descend(origin_depth, location);
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.state.borrow().can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.state.borrow().can_go_forward()
    }

    pub fn can_go_parent(&self) -> bool {
        self.state.borrow().can_go_parent()
            && self
                .active_location()
                .and_then(|location| location.parent())
                .is_some_and(|parent| self.source.allows_navigation(&parent))
    }

    pub fn back_locations(&self) -> Vec<Location> {
        self.state.borrow().back_locations()
    }

    pub fn back(self: &Rc<Self>) {
        self.back_to(1);
    }

    pub fn back_to(self: &Rc<Self>, steps: usize) {
        let target = self.state.borrow_mut().go_back_to(steps);
        if let Some(target) = target {
            self.restore_path(target);
        }
    }

    pub fn forward(self: &Rc<Self>) {
        let target = self.state.borrow_mut().go_forward();
        if let Some(target) = target {
            self.restore_path(target);
        }
    }

    pub fn parent(self: &Rc<Self>) {
        if !self.can_go_parent() {
            return;
        }
        let target = self.state.borrow_mut().go_parent();
        if let Some(target) = target {
            self.restore_path(target);
        }
    }

    /// Drops the columns before `depth`, keeping the ones after it, and leaves
    /// focus, the cursor and a multi-selection in what becomes the first column.
    /// False when nothing was re-rooted.
    pub fn make_first_column(self: &Rc<Self>, depth: usize) -> bool {
        if (depth..)
            .map_while(|depth| self.location_at(depth))
            .any(|location| !self.source.allows_navigation(&location))
        {
            return false;
        }
        let cursor = self.cursor_entry(depth).map(|entry| entry.location);
        let mut filled: Vec<_> = self
            .filled_entries(depth)
            .into_iter()
            .map(|entry| entry.location)
            .collect();
        let selection = if filled.len() > 1 {
            // The first listed target takes the cursor.
            if let Some(at) = filled
                .iter()
                .position(|location| Some(location) == cursor.as_ref())
            {
                filled[..=at].rotate_right(1);
            }
            LoadSelection::Target(filled)
        } else {
            cursor.map_or(LoadSelection::FirstEntry, LoadSelection::Prefer)
        };
        let target = self.state.borrow_mut().go_to_column(depth);
        let Some(target) = target else {
            return false;
        };
        self.restore_path_focused(target, 0, selection);
        true
    }

    pub(super) fn restore_path(self: &Rc<Self>, path: NavigationPath) {
        let preferred = path
            .locations()
            .last()
            .and_then(|destination| self.came_from_child(destination));
        let Some(depth) = path.locations().len().checked_sub(1) else {
            return;
        };
        let selection = preferred.map_or(LoadSelection::FirstEntry, LoadSelection::Prefer);
        self.restore_path_focused(path, depth, selection);
    }

    /// Restores `path` with focus in the column at `focus`, which selects
    /// `selection` once it loads.
    fn restore_path_focused(
        self: &Rc<Self>,
        path: NavigationPath,
        focus: usize,
        selection: LoadSelection,
    ) {
        let _update = self.navigation_update();
        self.cancel_deferred_reveal();
        if path
            .locations()
            .iter()
            .any(|location| !self.source.allows_navigation(location))
        {
            return;
        }
        self.bump_navigation_generation();
        self.emit(BrowserEvent::NavigationStarting { history: true });
        self.close_peek();
        self.loads.borrow_mut().clear();
        self.monitors.borrow_mut().clear();
        self.cancel_deferred_work();
        let loads: Vec<_> = path
            .locations()
            .iter()
            .cloned()
            .map(|location| {
                let request_id = self.new_request_id();
                (location, request_id)
            })
            .collect();
        self.state
            .borrow_mut()
            .restore(path, loads.iter().map(|(_, request_id)| *request_id));

        let active_depth = (focus < loads.len()).then_some(focus);
        if let Some(depth) = active_depth {
            let mut state = self.state.borrow_mut();
            state.focus_column(depth);
            select_on_load(&mut state, depth, selection);
        }
        self.emit(BrowserEvent::Reset);
        for (depth, (location, request_id)) in loads.into_iter().enumerate() {
            self.emit(BrowserEvent::ColumnAdded {
                depth,
                location: location.clone(),
            });
            self.start_load(depth, location, request_id);
        }
        if let Some(depth) = active_depth {
            self.emit(BrowserEvent::FocusChanged {
                depth,
                position: None,
                triggered_by_removal: false,
            });
        }
    }
}

/// Opening `location` as the column after `parent_depth`.
#[derive(Clone)]
pub(super) struct Descent {
    parent_depth: usize,
    location: Location,
    selection: LoadSelection,
    /// Keeps focus in the parent and skips validating the folder first.
    keep_parent_active: bool,
}

impl Descent {
    pub(super) fn new(parent_depth: usize, location: Location, selection: LoadSelection) -> Self {
        Self {
            parent_depth,
            location,
            selection,
            keep_parent_active: false,
        }
    }

    fn keeping_parent_active(self) -> Self {
        Self {
            keep_parent_active: true,
            ..self
        }
    }
}

/// A descent waiting for its folder to validate, dropped when the
/// navigation moved on or the parent column changed.
struct PendingDescent {
    generation: u64,
    parent_location: Option<Location>,
    descent: Descent,
}
