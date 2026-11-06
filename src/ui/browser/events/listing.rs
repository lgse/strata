// SPDX-License-Identifier: MIT

//! Column publication: navigation resets, listing batches, metadata, sorting and load completion.

use super::MAX_BULK_REVEAL_SELECTION;
use crate::app::{EntryInsertion, EntrySplice};
use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser::columns::{
    BoundRow, ColumnView, column_size_text, restore_column_cursor, set_column_busy,
    set_column_selections, set_filter_placeholder, stop_column_spinner, touch_source_model,
    update_empty_trash_sensitivity,
};
use crate::ui::browser_modes::BrowserMode;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

/// Shows the listing, or the empty page unless a delete animation still owns it.
fn present_entry_count(column: &ColumnView, empty: bool, defer_empty: bool) {
    if empty {
        if !defer_empty {
            column.presentation.show_empty();
        }
    } else {
        column.presentation.show_content();
    }
}

fn record_publication(entry_count: usize, render_started: Instant) {
    crate::metrics::mark_batch_rendered(entry_count, render_started);
    crate::metrics::record_stage(
        "ui-publication",
        render_started.elapsed().as_millis() as u64,
    );
}

/// A camera root publishes before its load finishes, so its rows attach on the first batch.
fn attach_camera_model(column: &ColumnView) {
    if column.selection.model().is_none() {
        column.filtered_model.set_model(Some(&column.model));
        column.selection.set_model(Some(&column.filtered_model));
        column.syncing_selection.set(false);
    }
}

fn camera_top_anchor(
    column: &ColumnView,
) -> Option<crate::ui::browser::camera_scroll::CameraTopAnchor> {
    crate::ui::browser::camera_scroll::CameraTopAnchor::capture(column.list.upcast_ref())
}

fn splice_insertions(column: &ColumnView, insertions: &[EntryInsertion]) {
    for insertion in insertions {
        // Touch before splice: the model notifies synchronously.
        touch_source_model(column);
        column
            .model
            .splice(insertion.position as u32, 0, insertion.entries.len() as u32);
    }
}

fn start_column_spinner(column: &ColumnView) {
    column.spinner.set_visible(true);
    column.spinner.start();
    set_column_busy(column, true);
}

/// Shows the folder as locked and describes it to assistive technology.
fn show_locked_folder_row(
    row: &gtk::Box,
    item: &gtk::ListItem,
    bound: &BoundRow,
    entry: &FileEntry,
) {
    if let Some(icon) = row
        .first_child()
        .and_downcast::<crate::ui::thumbnail::ThumbnailSlot>()
    {
        crate::ui::thumbnail::show_locked_folder(&icon);
    }
    if let Some(label) = bound.rename_label.upgrade() {
        crate::ui::accessibility::describe_entry(item, &label.label(), Some(entry));
    }
}

fn refill_bound_row(bound: &BoundRow, item: &gtk::ListItem, row: &gtk::Box, entry: &FileEntry) {
    if entry.is_locked_folder() {
        show_locked_folder_row(row, item, bound, entry);
    }
    let size = &bound.size;
    let text = column_size_text(Some(entry));
    let actively_renaming = bound.edit.is_editing();
    size.set_label(&text);
    size.set_visible(
        crate::ui::preferences::PreferenceManager::shared().columns_show_sizes()
            && !text.is_empty()
            && !actively_renaming,
    );
}

impl ViewState {
    pub(super) fn on_navigation_starting(&self) {
        self.forget_listing_search();
        self.suppress_scroll_after_drop.set(false);
        self.drop_active_depths.set(None);
    }

    pub(super) fn on_reset(self: &Rc<Self>) {
        self.suppress_scroll_after_drop.set(false);
        self.pending_new_entry.take();
        if self.pending_properties.borrow().as_ref() != self.browser.location_at(0).as_ref() {
            self.pending_properties.take();
        }
        self.pending_location_credentials.take();
        self.pending_archive_destination.take();
        self.pending_location_selection.take();
        self.remove_open_argument_status();
        // A fresh location is coming, so skip the exit animation.
        self.truncate_for_replacement(0);
    }

    fn remove_open_argument_status(&self) {
        let mut child = self.overlay.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.has_css_class("open-argument-status") {
                self.overlay.remove_overlay(&widget);
            }
        }
    }

    pub(super) fn on_columns_truncated(self: &Rc<Self>, len: usize, replacing: bool) {
        self.pending_new_entry.take();
        if replacing {
            self.swap_columns_from(len);
        } else {
            self.truncate(len);
        }
        self.sync_displayed_location();
    }

    pub(super) fn on_column_added(self: &Rc<Self>, depth: usize, location: &Location) {
        self.sync_displayed_location();
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            self.append_column(depth, location);
        }
        self.settle_connect_column(location);
    }

    pub(super) fn on_columns_relocated(self: &Rc<Self>, from_depth: usize) {
        if self
            .listing_search_depth()
            .is_some_and(|depth| depth >= from_depth)
        {
            self.forget_listing_search();
        }
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            self.rebuild_relocated_columns(from_depth);
        }
        self.sync_displayed_location();
    }

    fn rebuild_relocated_columns(self: &Rc<Self>, from_depth: usize) {
        let refocus = self
            .focused_column_depth()
            .is_some_and(|depth| depth >= from_depth);
        let active = self.browser.active_depth();
        self.rebuild_columns_from(from_depth);
        // A rename is not navigation: keep the user's horizontal viewport.
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().saturating_add(1));
        let Some(depth) = active else {
            return;
        };
        self.browser.set_active_column(depth);
        if refocus {
            self.browser.focus_active();
        }
    }

    pub(super) fn on_entries_inserted(&self, depth: usize, insertions: &[EntryInsertion]) {
        let render_started = Instant::now();
        let entry_count = insertions
            .iter()
            .map(|insertion| insertion.entries.len())
            .sum();
        let Some(column) = self.columns.borrow().get(depth).cloned() else {
            return;
        };
        let camera = self
            .browser
            .location_at(depth)
            .is_some_and(|location| location.is_camera_photo_root());
        let top = camera.then(|| camera_top_anchor(&column)).flatten();
        let reveals_content = entry_count > 0 && (!column.spinner.is_spinning() || camera);
        if reveals_content {
            column.presentation.show_content();
        }
        splice_insertions(&column, insertions);
        if camera && entry_count > 0 {
            attach_camera_model(&column);
        }
        let count = column.entry_count.get() + entry_count;
        column.entry_count.set(count);
        set_filter_placeholder(&column, count);
        update_empty_trash_sensitivity(&column, count);
        set_column_busy(&column, false);
        if let Some(top) = top {
            top.restore();
        }
        record_publication(entry_count, render_started);
    }

    pub(super) fn on_entries_replaced(&self, depth: usize, count: usize) {
        let Some(column) = self.columns.borrow().get(depth).cloned() else {
            return;
        };
        if count > 0 {
            column.presentation.show_content();
            set_column_busy(&column, false);
        }
        touch_source_model(&column);
        column.model.replace(count as u32);
        column.entry_count.set(count);
        set_filter_placeholder(&column, count);
        update_empty_trash_sensitivity(&column, count);
    }

    pub(super) fn on_entries_published(&self, depth: usize, position: usize, count: usize) {
        let render_started = Instant::now();
        let Some(column) = self.columns.borrow().get(depth).cloned() else {
            return;
        };
        if count > 0 && !column.spinner.is_spinning() {
            column.presentation.show_content();
        }
        touch_source_model(&column);
        column.model.splice(position as u32, 0, count as u32);
        let total = column.entry_count.get().saturating_add(count);
        column.entry_count.set(total);
        set_filter_placeholder(&column, total);
        update_empty_trash_sensitivity(&column, total);
        set_column_busy(&column, false);
        record_publication(count, render_started);
    }

    pub(super) fn on_metadata_filled(&self, depth: usize, updates: &[(usize, FileEntry)]) {
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return;
        }
        let Some(column) = self.columns.borrow().get(depth).cloned() else {
            return;
        };
        let filled: HashMap<usize, &FileEntry> = updates
            .iter()
            .map(|(position, entry)| (*position, entry))
            .collect();
        if filled.is_empty() {
            return;
        }
        column.bound_rows.borrow_mut().retain(|bound| {
            let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                return false;
            };
            let entry = column
                .map
                .source_position(item.position())
                .and_then(|position| filled.get(&position));
            if let Some(&entry) = entry {
                refill_bound_row(bound, &item, &row, entry);
            }
            true
        });
    }

    pub(super) fn on_sorting_started(&self, depth: usize) {
        self.overlay.set_cursor_from_name(Some("wait"));
        if let Some(column) = self.columns.borrow().get(depth) {
            crate::ui::accessibility::set_description(
                &column.spinner,
                Some(&crate::i18n::tr("Sorting…")),
            );
            start_column_spinner(column);
        }
    }

    pub(super) fn on_sorting_finished(self: &Rc<Self>, depth: usize) {
        self.finish_keyboard_refocus(
            crate::ui::browser::file_commands::KeyboardRefocus::Sort(depth),
            true,
        );
        self.overlay.set_cursor(None::<&gtk::gdk::Cursor>);
        if let Some(column) = self.columns.borrow().get(depth) {
            crate::ui::browser::pane_header::sync_column_sort_direction(
                &self.browser,
                depth,
                &column.sort_direction_button,
            );
            stop_column_spinner(column);
            crate::ui::accessibility::set_description(&column.spinner, None);
            set_column_busy(column, false);
        }
    }

    pub(super) fn on_entries_spliced(self: &Rc<Self>, depth: usize, splices: &[EntrySplice]) {
        self.splice_column_entries(depth, splices);
        // Hits whose files left the folder, whoever removed them.
        if splices.iter().any(|splice| splice.removed > 0) {
            self.prune_stale_search_results();
        }
        self.note_pending_rename_splices(depth, splices);
        self.reveal_pending_transfer_at(depth);
        if self.pending_archive_destination.borrow().is_some() {
            self.schedule_pending_archive_reveal(depth);
        }
    }

    fn splice_column_entries(&self, depth: usize, splices: &[EntrySplice]) {
        let defer_empty = self.delete_animation_defers_empty_state(depth);
        let restore_cursor = self.focused_column_depth() == Some(depth);
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        let mut count = column.entry_count.get();
        for splice in splices {
            touch_source_model(column);
            column.model.splice(
                splice.position as u32,
                splice.removed as u32,
                splice.entries.len() as u32,
            );
            count = count
                .saturating_sub(splice.removed)
                .saturating_add(splice.entries.len());
        }
        column.entry_count.set(count);
        set_filter_placeholder(column, count);
        // Recursive hits keep their own selection and cursor.
        let hits = column.recursive_search_active.get();
        if !hits {
            self.sync_column_selection(column, depth);
        }
        if restore_cursor && !hits {
            self.restore_focused_cursor(column, depth);
        }
        present_entry_count(column, count == 0, defer_empty);
        set_column_busy(column, false);
        update_empty_trash_sensitivity(column, count);
    }

    /// Selects the browser's positions for `depth` on the column's rows.
    fn sync_column_selection(&self, column: &ColumnView, depth: usize) {
        let positions: Vec<_> = self
            .browser
            .selected_positions(depth)
            .into_iter()
            .filter_map(|position| column.map.view_position(position))
            .collect();
        set_column_selections(column, &positions);
    }

    fn restore_focused_cursor(&self, column: &ColumnView, depth: usize) {
        let Some((focused_depth, position, _)) = self.browser.focused_item() else {
            return;
        };
        if focused_depth != depth {
            return;
        }
        if let Some(position) = column.map.view_position(position) {
            restore_column_cursor(column, position);
        }
    }

    fn schedule_pending_archive_reveal(self: &Rc<Self>, depth: usize) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.reveal_pending_archive_at(depth);
            }
        });
    }

    pub(super) fn on_column_refreshing(&self, depth: usize) {
        if let Some(column) = self.columns.borrow().get(depth) {
            start_column_spinner(column);
        }
    }

    pub(super) fn on_column_not_responding(&self, depth: usize, not_responding: bool) {
        if let Some(column) = self.columns.borrow().get(depth) {
            column.not_responding.set_visible(not_responding);
        }
    }

    pub(super) fn on_column_reloaded(&self, depth: usize) {
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        crate::ui::browser::pane_header::sync_column_sort_direction(
            &self.browser,
            depth,
            &column.sort_direction_button,
        );
        // Filters and searches outlive the monitor rescans of a busy
        // folder. Their hits come from the search, not this listing.
        let preserve_search = column.recursive_search_active.get();
        if !preserve_search {
            column.search_session.cancel();
            crate::ui::browser::collection::deactivate_recursive_search(
                &column.recursive_search_active,
                &column.search_results,
                &column.search_model,
                &column.filtered_model,
                &column.model,
            );
            column.syncing_selection.set(true);
            column.selection.set_model(None::<&gio::ListModel>);
        }
        touch_source_model(column);
        column.model.replace(0);
        column.entry_count.set(0);
        set_filter_placeholder(column, 0);
        column.truncated_hint.set_visible(false);
        start_column_spinner(column);
        if !preserve_search {
            column.presentation.show_loading();
        }
    }

    pub(super) fn on_hidden_toggled(&self, show_hidden: bool) {
        for column in self.columns.borrow().iter() {
            column.show_hidden.set(show_hidden);
            column.search_session.set_show_hidden(show_hidden);
            touch_source_model(column);
            column.filter.changed(gtk::FilterChange::Different);
        }
        self.mode_views.borrow().set_show_hidden(show_hidden);
    }

    pub(super) fn on_load_finished(self: &Rc<Self>, depth: usize, truncated: bool) {
        let defer_empty = self.delete_animation_defers_empty_state(depth);
        let archive_destination_loaded = self.archive_destination_loaded_at(depth);
        if archive_destination_loaded && self.mode_views.borrow().mode() == BrowserMode::Columns {
            self.browser.set_active_column(depth);
        }
        self.finish_column_load(depth, truncated, defer_empty);
        self.reveal_pending_transfer_at(depth);
        self.notify_filter_listing_loaded(depth);
        if archive_destination_loaded {
            self.select_pending_archive_names(depth);
        } else {
            self.select_pending_names_after_load(depth);
        }
    }

    fn archive_destination_loaded_at(&self, depth: usize) -> bool {
        !self.pending_select.borrow().is_empty()
            && self
                .pending_archive_destination
                .borrow()
                .as_ref()
                .is_some_and(|destination| {
                    self.browser.location_at(depth).as_ref() == Some(destination)
                })
    }

    fn finish_column_load(&self, depth: usize, truncated: bool, defer_empty: bool) {
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        if column.selection.model().is_none() {
            column.syncing_selection.set(true);
            column.filtered_model.set_model(Some(&column.model));
            column.selection.set_model(Some(&column.filtered_model));
        }
        let positions: Vec<u32> = self
            .browser
            .selected_positions(depth)
            .into_iter()
            .filter_map(|position| column.map.view_position(position))
            .collect();
        if !column.recursive_search_active.get() {
            set_column_selections(column, &positions);
        }
        stop_column_spinner(column);
        column.truncated_hint.set_visible(truncated);
        let count = column.entry_count.get();
        let empty = count == 0 && !column.recursive_search_active.get();
        present_entry_count(column, empty, defer_empty);
        set_column_busy(column, false);
        update_empty_trash_sensitivity(column, count);
    }

    fn select_pending_archive_names(self: &Rc<Self>, depth: usize) {
        let names = self.pending_select.take();
        if names.is_empty() {
            return;
        }
        let weak = Rc::downgrade(self);
        let destination = self.browser.location_at(depth);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.reveal_archive_names(depth, destination, names);
            }
        });
    }

    fn reveal_archive_names(
        self: &Rc<Self>,
        depth: usize,
        destination: Option<Location>,
        names: Vec<String>,
    ) {
        let still_loaded = self.browser.location_at(depth) == destination
            && self.pending_archive_destination.borrow().as_ref() == destination.as_ref();
        if !still_loaded {
            return;
        }
        if self.browser.select_entries_by_name_at(depth, &names) {
            self.reveal_focused_entry();
            self.pending_archive_destination.take();
        } else {
            self.pending_select.borrow_mut().extend(names);
        }
    }

    /// The active column takes the pending names, unless a Columns archive reveal still owns them.
    fn takes_pending_select_at(&self, depth: usize) -> bool {
        self.browser.active_depth() == Some(depth)
            && (self.mode_views.borrow().mode() != BrowserMode::Columns
                || self.pending_archive_destination.borrow().is_none())
    }

    fn select_pending_names_after_load(self: &Rc<Self>, depth: usize) {
        let names = if self.takes_pending_select_at(depth) {
            self.pending_select.take()
        } else {
            Vec::new()
        };
        let properties = self.pending_properties.borrow().is_some()
            && *self.pending_properties.borrow() == self.browser.location_at(depth);
        if properties {
            self.pending_properties.take();
        }
        if names.is_empty() && !properties {
            return;
        }
        let weak = Rc::downgrade(self);
        let destination = self.browser.location_at(depth);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade()
                && state.browser.location_at(depth) == destination
            {
                state.reveal_loaded_names(depth, &names, properties);
            }
        });
    }

    fn reveal_loaded_names(self: &Rc<Self>, depth: usize, names: &[String], properties: bool) {
        if !names.is_empty() {
            let reveal_names = if names.len() > MAX_BULK_REVEAL_SELECTION {
                &names[..1]
            } else {
                names
            };
            self.browser.select_entries_by_name_at(depth, reveal_names);
            self.reveal_focused_entry();
        }
        if self
            .pending_archive_destination
            .borrow()
            .as_ref()
            .is_some_and(|destination| self.browser.active_location().as_ref() == Some(destination))
        {
            self.pending_archive_destination.take();
        }
        if properties && let Some(entry) = self.browser.focused_entry() {
            self.show_entry_properties_at(entry, depth);
        }
    }

    pub(super) fn on_load_failed(&self, depth: usize, message: &str, offers_administrator: bool) {
        if let Some(location) = self.browser.location_at(depth) {
            crate::ui::preferences::PreferenceManager::shared().forget_missing_folder(&location);
        }
        self.notify_filter_listing_loaded(depth);
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        if column.selection.model().is_none() {
            column.filtered_model.set_model(Some(&column.model));
            column.selection.set_model(Some(&column.filtered_model));
            column.syncing_selection.set(false);
        }
        stop_column_spinner(column);
        column.presentation.show_error(
            &rust_i18n::t!(
                "Unable to read this directory\n%{message}",
                message = message
            ),
            offers_administrator,
        );
        set_column_busy(column, false);
    }
}
