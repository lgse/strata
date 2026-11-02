// SPDX-License-Identifier: MIT

//! Selection and cursor events, and the child column that mirrors the focused folder.

use crate::app::SelectionUpdate;
use crate::ui::browser::ViewState;
use crate::ui::browser::columns::{
    ColumnView, restore_column_cursor, scroll_column_to, select_all_in_column,
    set_column_selections,
};
use crate::ui::browser::listing_filter::Target;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::input_ownership::NavigationInput;
use crate::ui::preview::{FOCUS_PREVIEW_DELAY, preview_target};
use gtk::glib;
use std::rc::Rc;

/// One `SelectionSetChanged` event.
pub(super) struct SelectionChange<'a> {
    pub(super) depth: usize,
    pub(super) selection: &'a SelectionUpdate,
    pub(super) focused: usize,
    pub(super) take_focus: bool,
}

/// One `FocusChanged` event.
pub(super) struct FocusChange {
    pub(super) depth: usize,
    pub(super) position: Option<usize>,
    pub(super) triggered_by_removal: bool,
}

impl ViewState {
    pub(super) fn on_selection_set_changed(&self, change: &SelectionChange<'_>) {
        self.apply_selection_update(change);
        self.refresh_destination_style();
    }

    fn apply_selection_update(&self, change: &SelectionChange<'_>) {
        let columns = self.columns.borrow();
        let Some(column) = columns.get(change.depth) else {
            return;
        };
        match change.selection {
            SelectionUpdate::All => select_all_in_column(column),
            SelectionUpdate::Positions(positions) => {
                let filtered_positions: Vec<_> = positions
                    .iter()
                    .filter_map(|position| column.map.view_position(*position))
                    .collect();
                set_column_selections(column, &filtered_positions);
            }
        }
        // A background batch delivered for a column that already has a
        // selection re-fires this event; don't let it steal focus from
        // an in-progress rename (visible for slow network directories
        // that stream many batches). A pending creation still needs to scroll.
        if self.active_rename.borrow().is_none() {
            self.settle_selection_focus(column, change);
        }
    }

    fn settle_selection_focus(&self, column: &ColumnView, change: &SelectionChange<'_>) {
        let depth = change.depth;
        let camera_loading = self
            .browser
            .column_snapshot(depth)
            .is_some_and(|snapshot| snapshot.loading && snapshot.location.is_camera_photo_root());
        let scrolls =
            change.take_focus || (self.focused_column_depth() == Some(depth) && !camera_loading);
        if scrolls && let Some(focused) = column.map.view_position(change.focused) {
            scroll_column_to(column, focused);
        }
        if change.take_focus && self.mode_views.borrow().mode() == BrowserMode::Columns {
            // A cursor restore queued for the previous cursor must not pull
            // focus back from the newly selected entry.
            let generation = &column.cursor_restore_generation;
            generation.set(generation.get().wrapping_add(1));
            column.focus_surface();
        }
    }

    pub(super) fn on_focus_changed(self: &Rc<Self>, change: &FocusChange) {
        let column = self.columns.borrow().get(change.depth).cloned();
        if let Some(column) = column {
            let editing = self.active_rename.borrow().is_some();
            self.sync_focused_column_rows(&column, change, editing);
            if self.column_follows_cursor_focus(change.depth, editing) {
                self.focus_column_on_cursor(&column);
            }
            if self.reveals_focused_column(editing, change.triggered_by_removal) {
                self.reveal_column(column.shell);
            }
        }
        self.refresh_destination_style();
        self.mirror_focused_folder(change.depth, change.position, change.triggered_by_removal);
    }

    fn sync_focused_column_rows(&self, column: &ColumnView, change: &FocusChange, editing: bool) {
        // Recursive hits keep their own selection and cursor.
        let Some(filtered_position) = change
            .position
            .filter(|_| !column.recursive_search_active.get())
            .and_then(|position| column.map.view_position(position))
        else {
            return;
        };
        let positions: Vec<_> = self
            .browser
            .selected_positions(change.depth)
            .into_iter()
            .filter_map(|position| column.map.view_position(position))
            .collect();
        set_column_selections(column, &positions);
        if !editing && !self.suppress_focus_scroll.get() {
            scroll_column_to(column, filtered_position);
            restore_column_cursor(column, filtered_position);
        }
    }

    fn column_follows_cursor_focus(&self, depth: usize, editing: bool) -> bool {
        !editing
            && !self.cursor_keeps_focus.get()
            && self.mode_views.borrow().mode() == BrowserMode::Columns
            && self.browser.active_depth() == Some(depth)
            && !self.suppress_scroll_after_drop.get()
            && !self.outside_change_keeps_focus()
            && (!self.browser.focus_follows_background_load() || self.column_may_take_focus())
    }

    fn focus_column_on_cursor(&self, column: &ColumnView) {
        let hits = column
            .recursive_search_active
            .get()
            .then(|| Target::Column(Box::new(column.clone())));
        if !hits.is_some_and(|hits| self.focus_results_on_cursor(&hits)) {
            column.focus_surface();
        }
    }

    fn reveals_focused_column(&self, editing: bool, triggered_by_removal: bool) -> bool {
        !editing
            && self.mode_views.borrow().mode() == BrowserMode::Columns
            && !self.suppress_scroll_after_drop.get()
            && (triggered_by_removal
                || self.input_ownership.borrow().last_navigation == NavigationInput::Keyboard)
    }

    pub(in crate::ui::browser) fn mirror_focused_folder(
        self: &Rc<Self>,
        depth: usize,
        position: Option<usize>,
        force: bool,
    ) {
        if let Some(source) = self.pending_mirror.borrow_mut().take() {
            source.remove();
        }
        let Some(position) = position else {
            return;
        };
        if self.child_mirror_blocked(force) {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(FOCUS_PREVIEW_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.pending_mirror.borrow_mut().take();
            state.apply_child_mirror(depth, position, force);
        });
        self.pending_mirror.replace(Some(source));
    }

    /// Mirroring is a Columns keyboard feature that must not interrupt an edit or a visual range.
    fn child_mirror_blocked(&self, force: bool) -> bool {
        self.browser.child_mirror_suppressed()
            || self.browser.visual_kind().is_some()
            || !self.columns_mirror_selection.get()
            || self.active_rename.borrow().is_some()
            || self.pending_new_entry.borrow().is_some()
            || self.mode_views.borrow().mode() != BrowserMode::Columns
            || (!force
                && self.input_ownership.borrow().last_navigation != NavigationInput::Keyboard)
    }

    /// A filtered or still-validating column must not mirror: opening or closing the
    /// child column would end a 10xer range, and the validating folder takes the focus itself.
    fn child_mirror_blocked_at(&self, depth: usize, force: bool) -> bool {
        let filtered = self
            .columns
            .borrow()
            .get(depth)
            .is_some_and(|column| column.map.has_query());
        filtered
            || self.browser.pending_navigation_generation().is_some()
            || self.child_mirror_blocked(force)
    }

    pub(in crate::ui::browser) fn apply_child_mirror(
        self: &Rc<Self>,
        depth: usize,
        position: usize,
        force: bool,
    ) {
        if self.child_mirror_blocked_at(depth, force) {
            return;
        }
        let Some((focused_depth, focused_position, entry)) = self.browser.focused_item() else {
            return;
        };
        if (focused_depth, focused_position) != (depth, position) {
            return;
        }
        if entry.is_directory() {
            self.browser.show_child(depth, entry.location);
            return;
        }
        self.browser.close_column(depth + 1);
        if self.single_click_previews.get()
            && let Some(entry) = preview_target(Some(entry))
        {
            self.browser.request_automatic_preview(entry);
        }
    }
}
