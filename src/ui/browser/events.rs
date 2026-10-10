// SPDX-License-Identifier: MIT

//! Exhaustive browser event dispatch. Shared effects and column publication run before alternate
//! presentations consume the event; preserve that order when adding a feature handler.

use crate::app::{BrowserEvent, SelectionUpdate};
use crate::model::{FileEntry, Location};
use crate::services::LocationValidationError;
use crate::ui::browser::ViewState;
use crate::ui::browser::archive::extract_password_retry;
use crate::ui::browser::columns::{
    column_size_text, prune_missing_search_results, restore_column_cursor, scroll_column_to,
    select_all_in_column, set_column_busy, set_column_selections, set_filter_placeholder,
    stop_column_spinner, touch_source_model, update_empty_trash_sensitivity,
};
use crate::ui::browser::location::MountStrategy;
use crate::ui::browser::peek::append_peek_entries;
use crate::ui::browser::transfer::FinishedSendToCompletion;
use crate::ui::browser::trash::retryable_delete_entries;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::modal::{show_delete_error_dialog, show_error_dialog};
use crate::ui::preview::{FOCUS_PREVIEW_DELAY, preview_target};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

const MAX_BULK_REVEAL_SELECTION: usize = 64;

impl ViewState {
    pub(super) fn handle(self: &Rc<Self>, event: &BrowserEvent) {
        if matches!(
            event,
            BrowserEvent::NavigationStarting { .. }
                | BrowserEvent::Reset
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnsRelocated { .. }
                | BrowserEvent::EntriesInserted { .. }
                | BrowserEvent::EntriesReplaced { .. }
                | BrowserEvent::EntriesPublished { .. }
                | BrowserEvent::EntriesSpliced { .. }
                | BrowserEvent::SortingStarted { .. }
                | BrowserEvent::ColumnReloading { .. }
                | BrowserEvent::ColumnReloaded { .. }
                | BrowserEvent::ColumnRefreshing { .. }
                | BrowserEvent::HiddenToggled { .. }
                | BrowserEvent::FocusChanged { .. }
                | BrowserEvent::SelectionSetChanged { .. }
                | BrowserEvent::SelectionSynced { .. }
                | BrowserEvent::OpenRequested { .. }
        ) {
            self.cancel_click_rename();
        }
        match event {
            BrowserEvent::BackgroundOperation { request_id, event } => {
                self.handle_background_file_operation(*request_id, event);
                return;
            }
            BrowserEvent::SelectionSynced { .. } => return,
            BrowserEvent::NavigationStarting { .. } => {
                self.forget_listing_search();
                self.suppress_scroll_after_drop.set(false);
                self.drop_active_depths.set(None);
            }
            BrowserEvent::Reset => {
                self.suppress_scroll_after_drop.set(false);
                self.pending_new_entry.take();
                if self.pending_properties.borrow().as_ref() != self.browser.location_at(0).as_ref()
                {
                    self.pending_properties.take();
                }
                self.pending_location_credentials.take();
                self.pending_archive_destination.take();
                self.pending_location_selection.take();
                let mut child = self.overlay.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    if widget.has_css_class("open-argument-status") {
                        self.overlay.remove_overlay(&widget);
                    }
                }
                self.truncate(0);
            }
            BrowserEvent::ColumnsTruncated { len } => {
                self.pending_new_entry.take();
                self.truncate(*len);
                self.sync_active_location();
            }
            BrowserEvent::ColumnAdded { depth, location } => {
                self.set_location(location);
                if self.mode_views.borrow().mode() == BrowserMode::Columns {
                    self.append_column(*depth, location);
                }
            }
            BrowserEvent::ColumnsRelocated { from_depth } => {
                if self.mode_views.borrow().mode() == BrowserMode::Columns {
                    let refocus = self
                        .focused_column_depth()
                        .is_some_and(|depth| depth >= *from_depth);
                    let active = self.browser.active_depth();
                    self.rebuild_columns_from(*from_depth);
                    // A rename is not navigation: keep the user's horizontal viewport.
                    self.horizontal_scroll_generation
                        .set(self.horizontal_scroll_generation.get().saturating_add(1));
                    if let Some(depth) = active {
                        self.browser.set_active_column(depth);
                        if refocus {
                            self.browser.focus_active();
                        }
                    }
                }
                if let Some(location) = (0..)
                    .map_while(|depth| self.browser.location_at(depth))
                    .last()
                {
                    self.set_location(&location);
                }
            }
            BrowserEvent::EntriesInserted { depth, insertions } => {
                let render_started = Instant::now();
                let entry_count = insertions
                    .iter()
                    .map(|insertion| insertion.entries.len())
                    .sum();
                if let Some(column) = self.columns.borrow().get(*depth).cloned() {
                    let camera = self
                        .browser
                        .location_at(*depth)
                        .is_some_and(|location| location.is_camera_photo_root());
                    let top = camera
                        .then(|| {
                            super::camera_scroll::CameraTopAnchor::capture(column.list.upcast_ref())
                        })
                        .flatten();
                    if entry_count > 0 && (!column.spinner.is_spinning() || camera) {
                        column.presentation.show_content();
                    }
                    for insertion in insertions {
                        // Touch before splice: the model notifies synchronously.
                        touch_source_model(&column);
                        column.model.splice(
                            insertion.position as u32,
                            0,
                            insertion.entries.len() as u32,
                        );
                    }
                    if camera && entry_count > 0 && column.selection.model().is_none() {
                        column.filtered_model.set_model(Some(&column.model));
                        column.selection.set_model(Some(&column.filtered_model));
                        column.syncing_selection.set(false);
                    }
                    let count = column.entry_count.get() + entry_count;
                    column.entry_count.set(count);
                    set_filter_placeholder(&column, count);
                    update_empty_trash_sensitivity(&column, count);
                    set_column_busy(&column, false);
                    if let Some(top) = top {
                        top.restore();
                    }
                    crate::metrics::mark_batch_rendered(entry_count, render_started);
                    crate::metrics::record_stage(
                        "ui-publication",
                        render_started.elapsed().as_millis() as u64,
                    );
                }
            }
            BrowserEvent::EntriesReplaced { depth, count } => {
                if let Some(column) = self.columns.borrow().get(*depth).cloned() {
                    if *count > 0 {
                        column.presentation.show_content();
                        set_column_busy(&column, false);
                    }
                    touch_source_model(&column);
                    column.model.replace(*count as u32);
                    column.entry_count.set(*count);
                    set_filter_placeholder(&column, *count);
                    update_empty_trash_sensitivity(&column, *count);
                }
            }
            BrowserEvent::EntriesPublished {
                depth,
                position,
                count,
            } => {
                let render_started = Instant::now();
                if let Some(column) = self.columns.borrow().get(*depth).cloned() {
                    if *count > 0 && !column.spinner.is_spinning() {
                        column.presentation.show_content();
                    }
                    touch_source_model(&column);
                    column.model.splice(*position as u32, 0, *count as u32);
                    let total = column.entry_count.get().saturating_add(*count);
                    column.entry_count.set(total);
                    set_filter_placeholder(&column, total);
                    update_empty_trash_sensitivity(&column, total);
                    set_column_busy(&column, false);
                    crate::metrics::mark_batch_rendered(*count, render_started);
                    crate::metrics::record_stage(
                        "ui-publication",
                        render_started.elapsed().as_millis() as u64,
                    );
                }
            }
            BrowserEvent::MetadataFilled { depth, updates } => {
                if self.mode_views.borrow().mode() == BrowserMode::Columns
                    && let Some(column) = self.columns.borrow().get(*depth).cloned()
                {
                    let filled: HashMap<usize, &FileEntry> = updates
                        .iter()
                        .map(|(position, entry)| (*position, entry))
                        .collect();
                    if !filled.is_empty() {
                        column.bound_rows.borrow_mut().retain(|bound| {
                            let (Some(item), Some(_row)) =
                                (bound.item.upgrade(), bound.row.upgrade())
                            else {
                                return false;
                            };
                            let position = column.map.source_position(item.position());
                            if let Some(position) = position
                                && let Some(&entry) = filled.get(&position)
                            {
                                let size = &bound.size;
                                let text = column_size_text(Some(entry));
                                let actively_renaming = bound.edit.is_editing();
                                size.set_label(&text);
                                size.set_visible(!text.is_empty() && !actively_renaming);
                            }
                            true
                        });
                    }
                }
            }
            BrowserEvent::SortingStarted { depth } => {
                self.overlay.set_cursor_from_name(Some("wait"));
                if let Some(column) = self.columns.borrow().get(*depth) {
                    crate::ui::accessibility::set_description(
                        &column.spinner,
                        Some(&crate::i18n::tr("Sorting…")),
                    );
                    column.spinner.set_visible(true);
                    column.spinner.start();
                    set_column_busy(column, true);
                }
            }
            BrowserEvent::SortingFinished { depth } => {
                self.finish_keyboard_refocus(
                    super::file_commands::KeyboardRefocus::Sort(*depth),
                    true,
                );
                self.overlay.set_cursor(None::<&gtk::gdk::Cursor>);
                if let Some(column) = self.columns.borrow().get(*depth) {
                    super::pane_header::sync_column_sort_direction(
                        &self.browser,
                        *depth,
                        &column.sort_direction_button,
                    );
                    stop_column_spinner(column);
                    crate::ui::accessibility::set_description(&column.spinner, None);
                    set_column_busy(column, false);
                }
            }
            BrowserEvent::EntriesSpliced { depth, splices, .. } => {
                let defer_empty = self.delete_animation_defers_empty_state(*depth);
                let restore_cursor = self.focused_column_depth() == Some(*depth);
                if let Some(column) = self.columns.borrow().get(*depth) {
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
                        let positions: Vec<_> = self
                            .browser
                            .selected_positions(*depth)
                            .into_iter()
                            .filter_map(|position| column.map.view_position(position))
                            .collect();
                        set_column_selections(column, &positions);
                    }
                    if restore_cursor
                        && !hits
                        && let Some((focused_depth, position, _)) = self.browser.focused_item()
                        && focused_depth == *depth
                        && let Some(position) = column.map.view_position(position)
                    {
                        restore_column_cursor(column, position);
                    }
                    if count == 0 {
                        if !defer_empty {
                            column.presentation.show_empty();
                        }
                    } else {
                        column.presentation.show_content();
                    }
                    set_column_busy(column, false);
                    update_empty_trash_sensitivity(column, count);
                }
                // Hits whose files left the folder, whoever removed them.
                if splices.iter().any(|splice| splice.removed > 0) {
                    self.prune_stale_search_results();
                }
                self.note_pending_rename_splices(*depth, splices);
                self.reveal_pending_transfer_at(*depth);
                if self.pending_archive_destination.borrow().is_some() {
                    let weak = Rc::downgrade(self);
                    let depth = *depth;
                    glib::idle_add_local_once(move || {
                        if let Some(state) = weak.upgrade() {
                            state.reveal_pending_archive_at(depth);
                        }
                    });
                }
            }
            BrowserEvent::ColumnRefreshing { depth } => {
                if let Some(column) = self.columns.borrow().get(*depth) {
                    column.spinner.set_visible(true);
                    column.spinner.start();
                    set_column_busy(column, true);
                }
            }
            BrowserEvent::ColumnReloaded { depth } => {
                if let Some(column) = self.columns.borrow().get(*depth) {
                    super::pane_header::sync_column_sort_direction(
                        &self.browser,
                        *depth,
                        &column.sort_direction_button,
                    );
                    // Filters and searches outlive the monitor rescans of a busy
                    // folder. Their hits come from the search, not this listing.
                    let preserve_search = column.recursive_search_active.get();
                    if !preserve_search {
                        column.search_session.cancel();
                        super::collection::deactivate_recursive_search(
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
                    column.spinner.set_visible(true);
                    column.spinner.start();
                    set_column_busy(column, true);
                    if !preserve_search {
                        column.presentation.show_loading();
                    }
                }
            }
            BrowserEvent::HiddenToggled { show_hidden } => {
                for column in self.columns.borrow().iter() {
                    column.show_hidden.set(*show_hidden);
                    column.search_session.set_show_hidden(*show_hidden);
                    touch_source_model(column);
                    column.filter.changed(gtk::FilterChange::Different);
                }
                self.mode_views.borrow().set_show_hidden(*show_hidden);
            }
            BrowserEvent::LoadFinished { depth, truncated } => {
                let defer_empty = self.delete_animation_defers_empty_state(*depth);
                let archive_destination_loaded = !self.pending_select.borrow().is_empty()
                    && self
                        .pending_archive_destination
                        .borrow()
                        .as_ref()
                        .is_some_and(|destination| {
                            self.browser.location_at(*depth).as_ref() == Some(destination)
                        });
                if archive_destination_loaded
                    && self.mode_views.borrow().mode() == BrowserMode::Columns
                {
                    self.browser.set_active_column(*depth);
                }
                if let Some(column) = self.columns.borrow().get(*depth) {
                    if column.selection.model().is_none() {
                        column.syncing_selection.set(true);
                        column.filtered_model.set_model(Some(&column.model));
                        column.selection.set_model(Some(&column.filtered_model));
                    }
                    let positions: Vec<u32> = self
                        .browser
                        .selected_positions(*depth)
                        .into_iter()
                        .filter_map(|position| column.map.view_position(position))
                        .collect();
                    if !column.recursive_search_active.get() {
                        set_column_selections(column, &positions);
                    }
                    stop_column_spinner(column);
                    column.truncated_hint.set_visible(*truncated);
                    let count = column.entry_count.get();
                    if count == 0 && !column.recursive_search_active.get() {
                        if !defer_empty {
                            column.presentation.show_empty();
                        }
                    } else {
                        column.presentation.show_content();
                    }
                    set_column_busy(column, false);
                    update_empty_trash_sensitivity(column, count);
                }
                self.reveal_pending_transfer_at(*depth);
                if archive_destination_loaded {
                    let names = self.pending_select.take();
                    if !names.is_empty() {
                        let weak = Rc::downgrade(self);
                        let depth = *depth;
                        let destination = self.browser.location_at(depth);
                        glib::idle_add_local_once(move || {
                            if let Some(state) = weak.upgrade()
                                && state.browser.location_at(depth) == destination
                                && state.pending_archive_destination.borrow().as_ref()
                                    == destination.as_ref()
                            {
                                if state.browser.select_entries_by_name_at(depth, &names) {
                                    state.reveal_focused_entry();
                                    state.pending_archive_destination.take();
                                } else {
                                    state.pending_select.borrow_mut().extend(names);
                                }
                            }
                        });
                    }
                } else {
                    let names = if self.browser.active_depth() == Some(*depth)
                        && (self.mode_views.borrow().mode() != BrowserMode::Columns
                            || self.pending_archive_destination.borrow().is_none())
                    {
                        self.pending_select.take()
                    } else {
                        Vec::new()
                    };
                    let properties = self.pending_properties.borrow().is_some()
                        && *self.pending_properties.borrow() == self.browser.location_at(*depth);
                    if properties {
                        self.pending_properties.take();
                    }
                    if !names.is_empty() || properties {
                        let weak = Rc::downgrade(self);
                        let depth = *depth;
                        let destination = self.browser.location_at(depth);
                        glib::idle_add_local_once(move || {
                            if let Some(state) = weak.upgrade()
                                && state.browser.location_at(depth) == destination
                            {
                                if !names.is_empty() {
                                    let reveal_names = if names.len() > MAX_BULK_REVEAL_SELECTION {
                                        &names[..1]
                                    } else {
                                        &names[..]
                                    };
                                    state.browser.select_entries_by_name_at(depth, reveal_names);
                                    state.reveal_focused_entry();
                                }
                                if state
                                    .pending_archive_destination
                                    .borrow()
                                    .as_ref()
                                    .is_some_and(|destination| {
                                        state.browser.active_location().as_ref()
                                            == Some(destination)
                                    })
                                {
                                    state.pending_archive_destination.take();
                                }
                                if properties && let Some(entry) = state.browser.focused_entry() {
                                    state.show_entry_properties_at(entry, depth);
                                }
                            }
                        });
                    }
                }
            }
            BrowserEvent::LoadFailed { depth, message } => {
                if let Some(column) = self.columns.borrow().get(*depth) {
                    if column.selection.model().is_none() {
                        column.filtered_model.set_model(Some(&column.model));
                        column.selection.set_model(Some(&column.filtered_model));
                        column.syncing_selection.set(false);
                    }
                    stop_column_spinner(column);
                    column.presentation.show_error(&rust_i18n::t!(
                        "Unable to read this directory\n%{message}",
                        message = message
                    ));
                    set_column_busy(column, false);
                }
            }
            BrowserEvent::PeekStarted { location } => self.append_peek(location),
            BrowserEvent::PeekEntriesAdded { entries } => {
                if let Some(peek) = self.peek.borrow().as_ref() {
                    if !entries.is_empty() {
                        peek.presentation.show_content();
                    }
                    append_peek_entries(peek, entries.clone(), self.peek_behavior.item_limit);
                }
            }
            BrowserEvent::PeekFinished => {
                if let Some(peek) = self.peek.borrow().as_ref() {
                    peek.spinner.stop();
                    peek.spinner.set_visible(false);
                    if peek.entry_count.get() == 0 {
                        peek.presentation.show_empty();
                    } else {
                        peek.presentation.show_content();
                    }
                }
            }
            BrowserEvent::PeekFailed { message } => {
                if let Some(peek) = self.peek.borrow().as_ref() {
                    peek.spinner.stop();
                    peek.spinner.set_visible(false);
                    peek.presentation.show_error(&rust_i18n::t!(
                        "Unable to read this directory\n%{message}",
                        message = message
                    ));
                }
            }
            BrowserEvent::PeekClosed => self.close_peek_visual(),
            BrowserEvent::SelectionSetChanged {
                depth,
                selection,
                focused,
                take_focus,
            } => {
                if let Some(column) = self.columns.borrow().get(*depth) {
                    match selection {
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
                        let camera_loading =
                            self.browser
                                .column_snapshot(*depth)
                                .is_some_and(|snapshot| {
                                    snapshot.loading && snapshot.location.is_camera_photo_root()
                                });
                        if (*take_focus
                            || (self.focused_column_depth() == Some(*depth) && !camera_loading))
                            && let Some(focused) = column.map.view_position(*focused)
                        {
                            scroll_column_to(column, focused);
                        }
                        if *take_focus && self.mode_views.borrow().mode() == BrowserMode::Columns {
                            // A cursor restore queued for the previous cursor must not pull
                            // focus back from the newly selected entry.
                            let generation = &column.cursor_restore_generation;
                            generation.set(generation.get().wrapping_add(1));
                            column.focus_surface();
                        }
                    }
                }
                self.refresh_destination_style();
            }
            BrowserEvent::FocusChanged { depth, position } => {
                let column = self.columns.borrow().get(*depth).cloned();
                if let Some(column) = column {
                    let editing = self.active_rename.borrow().is_some();
                    // Recursive hits keep their own selection and cursor.
                    if let Some(filtered_position) = position
                        .filter(|_| !column.recursive_search_active.get())
                        .and_then(|position| column.map.view_position(position))
                    {
                        let positions: Vec<_> = self
                            .browser
                            .selected_positions(*depth)
                            .into_iter()
                            .filter_map(|position| column.map.view_position(position))
                            .collect();
                        set_column_selections(&column, &positions);
                        if !editing && !self.suppress_focus_scroll.get() {
                            scroll_column_to(&column, filtered_position);
                            restore_column_cursor(&column, filtered_position);
                        }
                    }
                    if !editing
                        && !self.cursor_keeps_focus.get()
                        && self.mode_views.borrow().mode() == BrowserMode::Columns
                        && self.browser.active_depth() == Some(*depth)
                        && !self.suppress_scroll_after_drop.get()
                        && !self.outside_change_keeps_focus()
                        && (!self.browser.focus_follows_background_load()
                            || self.column_may_take_focus())
                    {
                        column.focus_surface();
                    }
                    if !editing
                        && self.mode_views.borrow().mode() == BrowserMode::Columns
                        && !self.suppress_scroll_after_drop.get()
                        && self.input_ownership.borrow().last_navigation
                            == crate::ui::input_ownership::NavigationInput::Keyboard
                    {
                        self.reveal_column(column.shell);
                    }
                }
                self.refresh_destination_style();
                self.mirror_focused_folder(*depth, *position);
            }
            BrowserEvent::ColumnReloading { .. } | BrowserEvent::PreviewRequested { .. } => {}
            BrowserEvent::ExtractRequested { entry } => {
                if self.interactive {
                    self.extract_entry(entry.clone());
                }
            }
            BrowserEvent::OpenRequested { location } => {
                if self.interactive {
                    let position = self
                        .playback_handoff
                        .borrow()
                        .as_ref()
                        .and_then(|handoff| handoff(location));
                    super::desktop::open_location_at(
                        location,
                        position,
                        &self.overlay,
                        &self.browser,
                    );
                }
            }
            BrowserEvent::EntryCreated { location } => {
                self.rename_created_entry(location);
            }
            BrowserEvent::RenameCompleted { request_id } => {
                self.complete_pending_rename(*request_id);
                self.prune_stale_search_results();
                self.finish_keyboard_refocus(
                    super::file_commands::KeyboardRefocus::Rename(*request_id),
                    true,
                );
            }
            BrowserEvent::RenameAbandoned { request_id } => {
                self.abandon_pending_rename(*request_id);
            }
            BrowserEvent::RenameFailed {
                request_id,
                message,
            } => {
                self.fail_pending_rename_from_browser(*request_id);
                if let Some(request_id) = request_id {
                    self.finish_keyboard_refocus(
                        super::file_commands::KeyboardRefocus::Rename(*request_id),
                        false,
                    );
                }
                show_error_dialog(
                    &self.overlay,
                    &crate::i18n::tr("Unable to rename item"),
                    message,
                );
            }
            BrowserEvent::TransferStarted { total, moving } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    *total,
                    if *moving {
                        crate::assets::icons::FOLDER
                    } else {
                        crate::assets::icons::COPY
                    },
                    if *moving {
                        "Moving items"
                    } else {
                        "Copying items"
                    },
                    "Cancelling will not undo completed changes",
                    super::progress::file_operation_cancel(browser),
                );
                self.update_transfer_progress(0, 0, None, 0, None);
                if let Some(id) = self.browser.backgroundable_operation() {
                    self.dock_file_operation(id);
                }
            }
            BrowserEvent::TransferProgress {
                completed_items,
                completed_files,
                total_files,
                current_file,
                transferred_bytes,
                total_bytes,
            } => {
                self.file_progress()
                    .transfer_current_file
                    .replace(current_file.clone());
                self.update_transfer_progress(
                    *completed_items,
                    *completed_files,
                    *total_files,
                    *transferred_bytes,
                    *total_bytes,
                );
            }
            BrowserEvent::FlushingToDevice => self.show_device_flush_status(),
            BrowserEvent::TransferCancellationPending => show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Transfer cancellation pending"),
                &crate::i18n::tr(
                    "The device may still be writing. Wait for the transfer to finish or fail before starting another file operation. Do not unplug until you can safely eject it.",
                ),
            ),
            BrowserEvent::TransferFinished { moved_locations } => {
                if !moved_locations.is_empty() {
                    self.complete_cut_transfer(moved_locations);
                }
                // TransferFinished also fires on failure; defer feedback until TransferCompleted.
                if let Some(completion) = self.pending_send_to_completion.take() {
                    let progress_shown = self.file_progress().file_progress_view.borrow().is_some();
                    self.finished_send_to_completion
                        .replace(Some(FinishedSendToCompletion {
                            completion,
                            progress_shown,
                        }));
                }
                self.dismiss_file_operation_progress();
                self.prune_stale_search_results();
            }
            BrowserEvent::DeletionStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    *total,
                    crate::assets::icons::TRASH,
                    "Deleting items",
                    "Cancelling will not undo completed changes",
                    super::progress::file_operation_cancel(browser),
                );
                self.file_progress().deleting.set(true);
                if let Some(id) = self.browser.backgroundable_operation() {
                    self.dock_file_operation(id);
                }
            }
            BrowserEvent::DeletionProgress { completed, total } => {
                self.update_item_progress(*completed, *total);
            }
            BrowserEvent::DeletionFinished { succeeded } => {
                let dissolve = self.pending_delete_dissolve.take();
                if let Some((depth, _)) = dissolve.as_ref() {
                    self.deferred_delete_empty_depth.set(Some(*depth));
                }
                let has_animation =
                    dissolve.is_some() || self.pending_file_operation_animation.borrow().is_some();
                if has_animation {
                    let succeeded = *succeeded;
                    let weak = Rc::downgrade(self);
                    self.dismiss_file_operation_progress_then(move || {
                        glib::idle_add_local_once(move || {
                            let Some(state) = weak.upgrade() else {
                                return;
                            };
                            if succeeded {
                                state.play_pending_file_operation_animation();
                                if let Some((depth, dissolve)) = dissolve {
                                    let weak = Rc::downgrade(&state);
                                    dissolve.play(move || {
                                        if let Some(state) = weak.upgrade() {
                                            state.finish_delete_animation(depth);
                                        }
                                    });
                                }
                            } else {
                                state.clear_delete_animation();
                                if let Some((depth, _)) = dissolve {
                                    state.finish_delete_animation(depth);
                                }
                            }
                        });
                    });
                } else {
                    self.dismiss_file_operation_progress();
                }
                self.prune_stale_search_results();
            }
            BrowserEvent::RestorationStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    *total,
                    crate::assets::icons::FOLDER,
                    "Restoring items",
                    "Cancelling will not undo completed changes",
                    super::progress::file_operation_cancel(browser),
                );
            }
            BrowserEvent::RestorationProgress { completed, total } => {
                self.update_item_progress(*completed, *total);
            }
            BrowserEvent::RestorationFinished { succeeded } => {
                let succeeded = *succeeded;
                let weak = Rc::downgrade(self);
                self.dismiss_file_operation_progress_then(move || {
                    glib::idle_add_local_once(move || {
                        if let Some(state) = weak.upgrade() {
                            if succeeded {
                                state.play_pending_file_operation_animation();
                            } else {
                                state.clear_delete_animation();
                            }
                        }
                    });
                });
            }
            BrowserEvent::OperationFailed {
                message,
                password_failure,
            } => {
                self.suppress_scroll_after_drop.set(false);
                self.drop_active_depths.set(None);
                self.pending_new_entry.take();
                self.pending_send_to_completion.take();
                self.finished_send_to_completion.take();
                self.pending_archive_destination.take();
                let retry = self.pending_extract_retry.take();
                let message = message.clone();
                let password_failure = *password_failure;
                let weak = Rc::downgrade(self);
                self.dismiss_file_operation_progress_then(move || {
                    let Some(state) = weak.upgrade() else {
                        return;
                    };
                    state.clear_delete_animation();
                    if let Some((entry, destination)) = retry
                        && let Some(invalid_password) = extract_password_retry(password_failure)
                    {
                        let navigate_after_extract = state.pending_navigate.take();
                        state.show_extract_password_dialog(
                            entry,
                            destination,
                            invalid_password,
                            navigate_after_extract,
                        );
                    } else {
                        show_error_dialog(
                            &state.overlay,
                            &crate::i18n::tr("Unable to complete operation"),
                            &message,
                        );
                    }
                });
            }
            BrowserEvent::OperationCompletedWithErrors {
                message,
                retryable_locations,
                has_non_retryable_failures,
            } => {
                self.suppress_scroll_after_drop.set(false);
                self.drop_active_depths.set(None);
                self.pending_archive_destination.take();
                self.pending_send_to_completion.take();
                self.finished_send_to_completion.take();
                let retryable_entries = retryable_delete_entries(
                    self.pending_delete_entries.take(),
                    retryable_locations,
                );
                let message = message.clone();
                let has_non_retryable_failures = *has_non_retryable_failures;
                let weak = Rc::downgrade(self);
                self.dismiss_file_operation_progress_then(move || {
                    let Some(state) = weak.upgrade() else {
                        return;
                    };
                    if retryable_entries.is_empty() {
                        crate::ui::modal::show_partial_failure_dialog(&state.overlay, &message);
                    } else if has_non_retryable_failures {
                        let weak_state = Rc::downgrade(&state);
                        show_delete_error_dialog(
                            &state.overlay,
                            &message,
                            Rc::new(move || {
                                if let Some(state) = weak_state.upgrade() {
                                    state.show_trash_unavailable_confirmation(
                                        retryable_entries.clone(),
                                    );
                                }
                            }),
                        );
                    } else {
                        state.show_trash_unavailable_confirmation(retryable_entries);
                    }
                });
            }
            BrowserEvent::OperationCancelled {
                completed,
                failed,
                not_attempted,
                affected_locations,
            } => {
                self.suppress_scroll_after_drop.set(false);
                self.drop_active_depths.set(None);
                self.pending_archive_destination.take();
                self.pending_send_to_completion.take();
                self.finished_send_to_completion.take();
                let affected_locations = affected_locations.clone();
                let message = super::progress::cancelled_operation_summary(
                    *completed,
                    *failed,
                    *not_attempted,
                );
                let weak = Rc::downgrade(self);
                self.dismiss_file_operation_progress_then(move || {
                    if let Some(state) = weak.upgrade() {
                        state
                            .browser
                            .refresh_after_cancellation(&affected_locations);
                        show_error_dialog(
                            &state.overlay,
                            &crate::i18n::tr("Operation cancelled"),
                            &message,
                        );
                    }
                });
            }
            BrowserEvent::OperationRefreshRequired { depths } => {
                let browser = self.browser.clone();
                let depths = depths.clone();
                self.dismiss_file_operation_progress_then(move || {
                    browser.refresh_operation_columns(&depths);
                });
            }
            BrowserEvent::NavigationRejected {
                parent_depth,
                error,
            } => {
                self.handle_navigation_rejected(*parent_depth, error.clone());
            }
            BrowserEvent::EmptyTrashRequested => {
                self.load_trash_summary();
            }
            BrowserEvent::LocationNavigationRejected { error } => {
                let credentials = self.pending_location_credentials.take();
                match error {
                    LocationValidationError::NotMounted(location) => {
                        self.mount_then_navigate_with_credentials(
                            location.clone(),
                            MountStrategy::EnclosingVolume,
                            credentials,
                        );
                    }
                    LocationValidationError::Mountable(location) => {
                        self.mount_then_navigate_with_credentials(
                            location.clone(),
                            MountStrategy::Mountable,
                            credentials,
                        );
                    }
                    error => {
                        self.abandon_deferred_reveal();
                        show_error_dialog(
                            &self.overlay,
                            &crate::i18n::tr("Unable to open location"),
                            &error.message(),
                        );
                    }
                }
            }
            BrowserEvent::LocationRevealFailed { location } => show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Unable to select file"),
                &rust_i18n::t!(
                    "%{path} is not available in the loaded folder.",
                    path = location.display_path()
                ),
            ),
            BrowserEvent::ArchiveStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    *total,
                    crate::assets::icons::FILE_ARCHIVE,
                    if self.browser.backgroundable_operation().is_some() {
                        "Compressing items"
                    } else {
                        "Processing archive…"
                    },
                    "Cancelling will not undo completed changes",
                    super::progress::file_operation_cancel(browser),
                );
                self.file_progress()
                    .archive_compressing
                    .set(self.browser.backgroundable_operation().is_some());
                self.update_archive_progress(0, *total);
                if let Some(id) = self.browser.backgroundable_operation() {
                    self.dock_file_operation(id);
                }
            }
            BrowserEvent::ArchiveProgress { completed, total } => {
                self.update_archive_progress(*completed, *total);
            }
            BrowserEvent::ArchiveCompleted { select_name, .. } => {
                self.pending_extract_retry.replace(None);
                let extracted_elsewhere = self
                    .extract_destination
                    .take()
                    .is_some_and(|destination| self.browser.active_location() != Some(destination));
                if select_name.is_empty() {
                    self.pending_archive_destination.take();
                }
                if let Some(destination) = self.pending_navigate.take() {
                    let weak = Rc::downgrade(self);
                    let select_name = select_name.clone();
                    let navigation_generation = self.browser.navigation_generation();
                    self.dismiss_file_operation_progress_then(move || {
                        if let Some(state) = weak.upgrade()
                            && state.browser.navigation_generation() == navigation_generation
                        {
                            if !select_name.is_empty() {
                                state.pending_select.borrow_mut().push(select_name);
                            }
                            state.browser.navigate(destination);
                        }
                    });
                } else if !select_name.is_empty()
                    && let Some(destination) = self.pending_archive_destination.borrow().clone()
                {
                    let weak = Rc::downgrade(self);
                    let select_name = select_name.clone();
                    self.dismiss_file_operation_progress_then(move || {
                        glib::idle_add_local_once(move || {
                            let Some(state) = weak.upgrade() else {
                                return;
                            };
                            if state.pending_archive_destination.borrow().as_ref()
                                != Some(&destination)
                            {
                                return;
                            }
                            state.pending_select.borrow_mut().push(select_name);
                            let depth = if state.mode_views.borrow().mode() == BrowserMode::Columns
                            {
                                (0..state.columns.borrow().len()).find(|depth| {
                                    state.browser.location_at(*depth).as_ref() == Some(&destination)
                                })
                            } else {
                                state.browser.active_depth().filter(|depth| {
                                    state.browser.location_at(*depth).as_ref() == Some(&destination)
                                })
                            };
                            if let Some(depth) = depth {
                                state.reveal_pending_archive_at(depth);
                            } else {
                                state.reload_archive_destination(destination);
                            }
                        });
                    });
                } else {
                    let weak = Rc::downgrade(self);
                    let select_name = select_name.clone();
                    let navigation_generation = self.browser.navigation_generation();
                    self.dismiss_file_operation_progress_then(move || {
                        if let Some(state) = weak.upgrade()
                            && state.browser.navigation_generation() == navigation_generation
                        {
                            if !select_name.is_empty() && !extracted_elsewhere {
                                state.pending_select.borrow_mut().push(select_name);
                            }
                            state.browser.reload_active();
                        }
                    });
                }
            }
            BrowserEvent::TransferReveal {
                destination,
                locations,
            } => {
                let weak = Rc::downgrade(self);
                let destination = destination.clone();
                let locations = locations.clone();
                self.dismiss_file_operation_progress_then(move || {
                    if let Some(state) = weak.upgrade() {
                        state.apply_transfer_reveal(destination, locations);
                    }
                });
            }
            BrowserEvent::TransferCompleted => {
                let weak = Rc::downgrade(self);
                self.dismiss_file_operation_progress_then(move || {
                    if let Some(state) = weak.upgrade() {
                        state.complete_transfer_ui();
                    }
                });
            }
        }
        if Self::event_refreshes_active_path(event) {
            self.refresh_active_path_rows();
        }
        let defer_empty = match event {
            BrowserEvent::EntriesReplaced { depth, .. }
            | BrowserEvent::EntriesSpliced { depth, .. }
            | BrowserEvent::LoadFinished { depth, .. } => {
                self.delete_animation_defers_empty_state(*depth)
            }
            _ => false,
        };
        self.mode_views
            .borrow_mut()
            .handle_with_deferred_empty(event, defer_empty);
        self.reconcile_pending_rename();
        match event {
            BrowserEvent::ColumnAdded { depth, .. } | BrowserEvent::ColumnReloaded { depth } => {
                self.note_pending_rename_refresh(*depth);
            }
            _ => {}
        }
        if matches!(
            event,
            BrowserEvent::LoadFinished { .. } | BrowserEvent::LoadFailed { .. }
        ) {
            let depth = match event {
                BrowserEvent::LoadFinished { depth, .. }
                | BrowserEvent::LoadFailed { depth, .. } => *depth,
                _ => unreachable!(),
            };
            self.reconcile_pending_rename_after_load(depth);
        }
    }

    fn reveal_pending_archive_at(self: &Rc<Self>, depth: usize) {
        if self
            .pending_archive_destination
            .borrow()
            .as_ref()
            .is_none_or(|destination| self.browser.location_at(depth).as_ref() != Some(destination))
        {
            return;
        }
        let names = self.pending_select.take();
        if names.is_empty() {
            return;
        }
        if self.browser.select_entries_by_name_at(depth, &names) {
            if self.mode_views.borrow().mode() == BrowserMode::Columns {
                self.browser.set_active_column(depth);
            }
            self.reveal_focused_entry();
            self.pending_archive_destination.take();
        } else {
            self.pending_select.borrow_mut().extend(names);
        }
    }

    fn apply_transfer_reveal(self: &Rc<Self>, destination: Location, mut locations: Vec<Location>) {
        if locations.len() > MAX_BULK_REVEAL_SELECTION {
            locations.truncate(1);
        }
        if let Some(depth) = self.browser.open_depth(&destination)
            && self.browser.column_has_monitor(depth)
        {
            self.prepare_reveal(&destination);
            if self
                .browser
                .reveal_monitored_locations(destination.clone(), &locations)
            {
                self.reveal_focused_entry();
            }
            if !self.browser.lists_every_target(depth, &locations) {
                self.pending_location_selection
                    .replace(Some((destination, locations)));
            }
            return;
        }
        let parent_depth = (self.mode_views.borrow().mode() == BrowserMode::Columns
            && self.browser.open_depth(&destination).is_none())
        .then(|| {
            destination
                .parent()
                .and_then(|parent| self.browser.open_depth(&parent))
        })
        .flatten();
        match parent_depth {
            Some(parent_depth) => {
                self.clear_pending_reveal_requests();
                self.browser
                    .descend_revealing(parent_depth, destination, locations);
            }
            None => self.reveal_locations(destination, locations, false),
        }
    }

    fn complete_transfer_ui(self: &Rc<Self>) {
        self.suppress_scroll_after_drop.set(false);
        if let Some(finished) = self.finished_send_to_completion.take()
            && !finished.progress_shown
        {
            self.show_send_to_success(
                &finished.completion.device_name,
                finished.completion.item_count,
            );
        }
        if let Some((source_depth, destination_depth)) = self.drop_active_depths.replace(None) {
            let weak = Rc::downgrade(self);
            glib::idle_add_local_once(move || {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if state.browser.active_depth() == Some(destination_depth)
                    && state.browser.location_at(source_depth).is_some()
                {
                    state.browser.set_active_column(source_depth);
                    state.browser.focus_active();
                }
            });
        }
        if let Some(destination) = self.pending_navigate.take() {
            self.browser.navigate(destination);
        }
    }

    fn reload_archive_destination(&self, destination: crate::model::Location) {
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            let depth = (0..self.columns.borrow().len())
                .find(|depth| self.browser.location_at(*depth).as_ref() == Some(&destination));
            if let Some(depth) = depth {
                self.browser.set_active_column(depth);
                self.browser.retry_column(depth);
            } else {
                self.browser.navigate(destination);
            }
        } else if self.browser.active_location().as_ref() == Some(&destination) {
            self.browser.reload_active();
        } else {
            self.browser.navigate(destination);
        }
    }

    fn clear_pending_reveal_requests(&self) {
        self.pending_archive_destination.take();
        self.pending_navigate.take();
        self.pending_select.take();
        self.pending_properties.take();
        self.pending_location_selection.take();
    }

    fn prepare_reveal(&self, directory: &Location) {
        self.clear_pending_reveal_requests();
        if let Some(depth) = self.browser.open_depth(directory) {
            if let Some(column) = self.columns.borrow().get(depth) {
                column.filter_entry.set_text("");
            }
            self.mode_views.borrow().clear_filter(depth);
        }
    }

    pub(super) fn reveal_locations(
        self: &Rc<Self>,
        directory: Location,
        targets: Vec<Location>,
        properties: bool,
    ) {
        self.prepare_reveal(&directory);
        if properties {
            self.pending_properties.replace(Some(directory.clone()));
        }
        if !self.browser.reveal_locations(directory, targets) {
            return;
        }
        // In-place selections have no load completion to trigger these actions.
        self.reveal_focused_entry();
        if self.pending_properties.take().is_some()
            && let Some((depth, _, entry)) = self.browser.focused_item()
        {
            self.show_entry_properties_at(entry, depth);
        }
    }

    pub(super) fn abandon_deferred_reveal(&self) {
        self.browser.cancel_deferred_reveal();
        self.pending_properties.take();
    }

    fn reveal_pending_transfer_at(self: &Rc<Self>, depth: usize) -> bool {
        let (selected, complete) = {
            let pending = self.pending_location_selection.borrow();
            let Some((destination, locations)) = pending.as_ref() else {
                return false;
            };
            if self.browser.location_at(depth).as_ref() != Some(destination) {
                return false;
            }
            (
                self.browser.select_entries_by_location_at(depth, locations),
                self.browser.lists_every_target(depth, locations),
            )
        };
        if complete {
            self.pending_location_selection.take();
        }
        if selected {
            self.reveal_focused_entry();
        }
        selected
    }

    pub(super) fn reveal_focused_entry(self: &Rc<Self>) {
        let Some((depth, position, _)) = self.browser.focused_item() else {
            return;
        };
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            let column = self.columns.borrow().get(depth).cloned();
            let Some(column) = column else {
                return;
            };
            if let Some(position) = column.map.view_position(position) {
                let rows = column.bound_rows.clone();
                super::collection::reveal_collection_after_layout(
                    column.list.upcast_ref(),
                    position,
                    Rc::new(move |visit| {
                        rows.borrow_mut().retain(|bound| {
                            let (Some(item), Some(row)) =
                                (bound.item.upgrade(), bound.row.upgrade())
                            else {
                                return false;
                            };
                            visit(item.position(), row.upcast_ref());
                            true
                        });
                    }),
                );
                self.reveal_column(column.shell);
            }
        } else {
            self.mode_views
                .borrow()
                .reveal_selected_entry(depth, position);
        }
    }

    pub(super) fn play_pending_delete_dissolve(self: &Rc<Self>, succeeded: bool) {
        self.delete_dissolve_request.set(None);
        let Some((depth, dissolve)) = self.pending_delete_dissolve.take() else {
            return;
        };
        self.deferred_delete_empty_depth.set(Some(depth));
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if succeeded {
                let weak = Rc::downgrade(&state);
                dissolve.play(move || {
                    if let Some(state) = weak.upgrade() {
                        state.finish_delete_animation(depth);
                    }
                });
            } else {
                state.finish_delete_animation(depth);
            }
        });
    }

    pub(super) fn settle_pending_delete_dissolve(&self) {
        self.delete_dissolve_request.set(None);
        let Some((depth, _)) = self.pending_delete_dissolve.take() else {
            return;
        };
        self.deferred_delete_empty_depth.set(Some(depth));
        self.finish_delete_animation(depth);
    }

    fn finish_delete_animation(&self, depth: usize) {
        if self.deferred_delete_empty_depth.get() != Some(depth) {
            return;
        }
        self.deferred_delete_empty_depth.set(None);
        if self.delete_animation_defers_empty_state(depth) {
            return;
        }
        if let Some(column) = self.columns.borrow().get(depth)
            && column.entry_count.get() == 0
            && !column.spinner.is_spinning()
        {
            column.presentation.show_empty_if_ready();
        }
        self.mode_views.borrow().show_empty_if_empty(depth);
    }

    pub(super) fn prune_stale_search_results(&self) {
        let columns = self.columns.borrow().clone();
        let mut changed = false;
        for column in &columns {
            changed |= prune_missing_search_results(column);
        }
        if changed {
            self.notify_search_selection_changed();
        }
        self.mode_views.borrow().prune_stale_search_results();
    }

    pub(super) fn mirror_focused_folder(self: &Rc<Self>, depth: usize, position: Option<usize>) {
        if let Some(source) = self.pending_mirror.borrow_mut().take() {
            source.remove();
        }
        let Some(position) = position else {
            return;
        };
        if self.browser.child_mirror_suppressed()
            || self.browser.visual_kind().is_some()
            || !self.columns_mirror_selection.get()
            || self.active_rename.borrow().is_some()
            || self.pending_new_entry.borrow().is_some()
            || self.mode_views.borrow().mode() != BrowserMode::Columns
            || self.input_ownership.borrow().last_navigation
                != crate::ui::input_ownership::NavigationInput::Keyboard
        {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(FOCUS_PREVIEW_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.pending_mirror.borrow_mut().take();
            state.apply_child_mirror(depth, position);
        });
        self.pending_mirror.replace(Some(source));
    }

    fn apply_child_mirror(self: &Rc<Self>, depth: usize, position: usize) {
        let filtered = self
            .columns
            .borrow()
            .get(depth)
            .is_some_and(|column| column.map.has_query());
        // Opening or closing the child column would end a 10xer range.
        if filtered
            || self.browser.child_mirror_suppressed()
            || self.browser.visual_kind().is_some()
            || !self.columns_mirror_selection.get()
            || self.active_rename.borrow().is_some()
            || self.pending_new_entry.borrow().is_some()
            || self.mode_views.borrow().mode() != BrowserMode::Columns
            || self.input_ownership.borrow().last_navigation
                != crate::ui::input_ownership::NavigationInput::Keyboard
        {
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
        } else {
            self.browser.close_column(depth + 1);
            if self.single_click_previews.get()
                && let Some(entry) = preview_target(Some(entry))
            {
                self.browser.request_automatic_preview(entry);
            }
        }
    }

    fn event_refreshes_active_path(event: &BrowserEvent) -> bool {
        matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnsRelocated { .. }
                | BrowserEvent::FocusChanged { .. }
                | BrowserEvent::SelectionSetChanged { .. }
                | BrowserEvent::EntriesInserted { .. }
                | BrowserEvent::EntriesPublished { .. }
                | BrowserEvent::EntriesSpliced { .. }
                | BrowserEvent::EntriesReplaced { .. }
        )
    }
}
