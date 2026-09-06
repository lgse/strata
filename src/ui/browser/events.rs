// SPDX-License-Identifier: GPL-3.0-or-later

//! Exhaustive browser event dispatch. Shared effects and column publication run before alternate
//! presentations consume the event; preserve that order when adding a feature handler.

use crate::app::BrowserEvent;
use crate::model::FileEntry;
use crate::services::LocationValidationError;
use crate::ui::browser::ViewState;
use crate::ui::browser::columns::{
    column_size_text, scroll_column_to, set_column_busy, set_column_selection,
    set_column_selections, set_filter_placeholder, stop_column_spinner, touch_source_model,
    update_empty_trash_sensitivity,
};
use crate::ui::browser::desktop::open_location;
use crate::ui::browser::entry::item_count_label;
use crate::ui::browser::location::MountStrategy;
use crate::ui::browser::peek::append_peek_entries;
use crate::ui::browser::trash::retryable_delete_entries;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::modal::{
    show_delete_error_dialog, show_error_dialog, show_error_dialog_after_close,
};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

impl ViewState {
    pub(super) fn handle(self: &Rc<Self>, event: &BrowserEvent) {
        match event {
            BrowserEvent::Reset => {
                self.pending_location_credentials.take();
                self.truncate(0);
            }
            BrowserEvent::ColumnsTruncated { len } => {
                self.truncate(*len);
                self.sync_active_location();
            }
            BrowserEvent::ColumnAdded { depth, location } => {
                self.set_location(location);
                if self.mode_views.borrow().mode() == BrowserMode::Columns {
                    self.append_column(*depth, location);
                    // A pointer click that opens a folder leaves the mouse on
                    // the parent column, so paste would target the folder's
                    // parent instead of the folder itself. Follow the newly
                    // opened column while the pointer owns navigation.
                    if self.input_ownership.borrow().last_navigation
                        == super::super::input_ownership::NavigationInput::Pointer
                        && self.hovered_column.get() == depth.checked_sub(1)
                    {
                        self.hovered_column.set(Some(*depth));
                        self.refresh_destination_style();
                    }
                }
            }
            BrowserEvent::EntriesInserted { depth, insertions } => {
                let render_started = Instant::now();
                let entry_count = insertions
                    .iter()
                    .map(|insertion| insertion.entries.len())
                    .sum();
                if let Some(column) = self.columns.borrow().get(*depth).cloned() {
                    if entry_count > 0 && !column.spinner.is_spinning() {
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
                    let count = column.entry_count.get() + entry_count;
                    column.entry_count.set(count);
                    set_filter_placeholder(&column, count);
                    update_empty_trash_sensitivity(&column, count);
                    set_column_busy(&column, false);
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
                for (_, entry) in updates.iter() {
                    crate::ui::thumbnail::note_metadata_entry(entry);
                }
                if self.mode_views.borrow().mode() == BrowserMode::Columns
                    && let Some(column) = self.columns.borrow().get(*depth).cloned()
                {
                    let filled: HashMap<usize, &FileEntry> = updates
                        .iter()
                        .map(|(position, entry)| (*position, entry))
                        .collect();
                    if !filled.is_empty() {
                        column.bound_rows.borrow_mut().retain(|bound| {
                            let (Some(item), Some(row)) =
                                (bound.item.upgrade(), bound.row.upgrade())
                            else {
                                return false;
                            };
                            let position = column.map.source_position(item.position());
                            if let Some(position) = position
                                && let Some(&entry) = filled.get(&position)
                                && let Some(size) = row
                                    .first_child()
                                    .and_downcast::<gtk::Image>()
                                    .and_then(|icon| icon.next_sibling())
                                    .and_then(|middle| middle.downcast::<gtk::Overlay>().ok())
                                    .and_then(|middle| middle.last_child())
                                    .and_downcast::<gtk::Label>()
                            {
                                let text = column_size_text(Some(entry));
                                let actively_renaming = self
                                    .active_rename
                                    .borrow()
                                    .as_ref()
                                    .is_some_and(|rename| rename.size == size);
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
                    column.spinner.set_tooltip_text(Some("Sorting…"));
                    column.spinner.set_visible(true);
                    column.spinner.start();
                    set_column_busy(column, true);
                }
            }
            BrowserEvent::SortingFinished { depth } => {
                self.overlay.set_cursor(None::<&gtk::gdk::Cursor>);
                if let Some(column) = self.columns.borrow().get(*depth) {
                    stop_column_spinner(column);
                    column.spinner.set_tooltip_text(None);
                    set_column_busy(column, false);
                }
            }
            BrowserEvent::EntriesSpliced {
                depth,
                splices,
                selected,
            } => {
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
                    set_column_selection(
                        column,
                        selected
                            .and_then(|position| column.map.view_position(position))
                            .unwrap_or(gtk::INVALID_LIST_POSITION),
                    );
                    if count == 0 {
                        column.presentation.show_empty();
                    } else {
                        column.presentation.show_content();
                    }
                    set_column_busy(column, false);
                    update_empty_trash_sensitivity(column, count);
                }
            }
            BrowserEvent::ColumnReloaded { depth } => {
                if let Some(column) = self.columns.borrow().get(*depth) {
                    column.search_handle.borrow_mut().take();
                    column
                        .search_generation
                        .set(column.search_generation.get().saturating_add(1));
                    column.search_results.borrow_mut().clear();
                    column
                        .search_model
                        .splice(0, column.search_model.n_items(), &[]);
                    column.filter_entry.set_text("");
                    column.syncing_selection.set(true);
                    column.selection.set_model(None::<&gio::ListModel>);
                    touch_source_model(column);
                    column.model.replace(0);
                    column.entry_count.set(0);
                    set_filter_placeholder(column, 0);
                    column.truncated_hint.set_visible(false);
                    column.spinner.set_visible(true);
                    column.spinner.start();
                    set_column_busy(column, true);
                    column.presentation.show_loading();
                }
            }
            BrowserEvent::HiddenToggled { show_hidden } => {
                for column in self.columns.borrow().iter() {
                    column.show_hidden.set(*show_hidden);
                    touch_source_model(column);
                    column.filter.changed(gtk::FilterChange::Different);
                }
                self.mode_views.borrow().set_show_hidden(*show_hidden);
            }
            BrowserEvent::LoadFinished { depth, truncated } => {
                if let Some(column) = self.columns.borrow().get(*depth) {
                    if column.selection.model().is_none() {
                        column.filtered_model.set_model(Some(&column.model));
                        column.selection.set_model(Some(&column.filtered_model));
                        column.syncing_selection.set(false);
                    }
                    stop_column_spinner(column);
                    column.truncated_hint.set_visible(*truncated);
                    let count = column.entry_count.get();
                    if count == 0 {
                        column.presentation.show_empty();
                    } else {
                        column.presentation.show_content();
                    }
                    set_column_busy(column, false);
                    update_empty_trash_sensitivity(column, count);
                }
                if self.browser.active_depth() == Some(*depth) {
                    let names = self.pending_select.take();
                    let properties = self.pending_select_properties.replace(false);
                    if !names.is_empty() {
                        let weak = Rc::downgrade(self);
                        glib::idle_add_local_once(move || {
                            if let Some(state) = weak.upgrade() {
                                state.browser.select_entries_by_name(&names);
                                if properties && let Some(entry) = state.browser.focused_entry() {
                                    state.show_entry_properties(entry);
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
                    column
                        .presentation
                        .show_error(&format!("Unable to read this directory\n{message}"));
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
                    peek.presentation
                        .show_error(&format!("Unable to read this directory\n{message}"));
                }
            }
            BrowserEvent::PeekClosed => self.close_peek_visual(),
            BrowserEvent::SelectionSetChanged {
                depth,
                positions,
                focused,
                take_focus,
            } => {
                if let Some(column) = self.columns.borrow().get(*depth) {
                    let filtered_positions: Vec<_> = positions
                        .iter()
                        .filter_map(|position| column.map.view_position(*position))
                        .collect();
                    set_column_selections(column, &filtered_positions);
                    // A background batch delivered for a column that already has a
                    // selection re-fires this event; don't let it steal focus from
                    // an in-progress New Folder/File prompt or rename (visible for
                    // slow network directories that stream many batches).
                    if self.active_rename.borrow().is_none()
                        && self.active_new_entry.borrow().is_none()
                    {
                        if (*take_focus || self.focused_column_depth() == Some(*depth))
                            && let Some(focused) = column.map.view_position(*focused)
                        {
                            scroll_column_to(column, focused);
                        }
                        if *take_focus && self.mode_views.borrow().mode() == BrowserMode::Columns {
                            column.list.grab_focus();
                        }
                    }
                }
            }
            BrowserEvent::FocusChanged { depth, position } => {
                if let Some(column) = self.columns.borrow().get(*depth) {
                    let editing = self.active_rename.borrow().is_some()
                        || self.active_new_entry.borrow().is_some();
                    if let Some(filtered_position) =
                        position.and_then(|position| column.map.view_position(position))
                    {
                        let positions: Vec<_> = self
                            .browser
                            .selected_positions(*depth)
                            .into_iter()
                            .filter_map(|position| column.map.view_position(position))
                            .collect();
                        set_column_selections(column, &positions);
                        if !editing {
                            scroll_column_to(column, filtered_position);
                        }
                    }
                    if !editing
                        && self.mode_views.borrow().mode() == BrowserMode::Columns
                        && !column.list.grab_focus()
                    {
                        column.presentation.stack.grab_focus();
                    }
                }
            }
            BrowserEvent::PreviewRequested { .. } => {}
            BrowserEvent::OpenRequested { location } => {
                if self.interactive {
                    open_location(location, &self.overlay);
                }
            }
            BrowserEvent::RenameCompleted => {
                self.cancel_rename();
                self.browser.focus_active();
            }
            BrowserEvent::RenameFailed { message } => {
                if let Some(rename) = self.active_rename.borrow().as_ref() {
                    rename.field.set_sensitive(true);
                    rename.field.add_css_class("error");
                    rename.field.set_tooltip_text(Some(message));
                    rename.field.grab_focus();
                }
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
                    Rc::new(move || browser.cancel_file_operation()),
                );
                self.update_transfer_progress(0, 0, None);
            }
            BrowserEvent::TransferProgress {
                completed_items,
                transferred_bytes,
                total_bytes,
            } => {
                self.update_transfer_progress(*completed_items, *transferred_bytes, *total_bytes);
            }
            BrowserEvent::TransferFinished { moved_locations } => {
                if !moved_locations.is_empty() {
                    self.complete_cut_transfer(moved_locations);
                }
                self.dismiss_file_operation_progress();
            }
            BrowserEvent::DeletionStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    *total,
                    crate::assets::icons::TRASH,
                    "Deleting items",
                    "Cancelling will not undo completed changes",
                    Rc::new(move || browser.cancel_file_operation()),
                );
            }
            BrowserEvent::DeletionProgress { completed, total } => {
                self.update_item_progress(*completed, *total);
            }
            BrowserEvent::DeletionFinished => self.dismiss_file_operation_progress(),
            BrowserEvent::RestorationStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    *total,
                    crate::assets::icons::FOLDER,
                    "Restoring items",
                    "Cancelling will not undo completed changes",
                    Rc::new(move || browser.cancel_file_operation()),
                );
            }
            BrowserEvent::RestorationProgress { completed, total } => {
                self.update_item_progress(*completed, *total);
            }
            BrowserEvent::RestorationFinished => self.dismiss_file_operation_progress(),
            BrowserEvent::OperationFailed { message } => {
                self.dismiss_file_operation_progress();
                let retry = self.pending_extract_retry.take();
                if let Some((entry, dest)) = retry {
                    let lower = message.to_lowercase();
                    if lower.contains("password") || lower.contains("encrypt") {
                        self.show_extract_password_dialog(entry, dest);
                        return;
                    }
                }
                show_error_dialog(&self.overlay, "Unable to complete operation", message);
            }
            BrowserEvent::OperationCompletedWithErrors {
                message,
                retryable_locations,
                has_non_retryable_failures,
            } => {
                let retryable_entries = retryable_delete_entries(
                    self.pending_delete_entries.take(),
                    retryable_locations,
                );
                if retryable_entries.is_empty() {
                    show_error_dialog(&self.overlay, "Completed with errors", message);
                } else if *has_non_retryable_failures {
                    let weak_state = Rc::downgrade(self);
                    show_delete_error_dialog(
                        &self.overlay,
                        message,
                        Rc::new(move || {
                            if let Some(state) = weak_state.upgrade() {
                                state.show_delete_confirmation(retryable_entries.clone());
                            }
                        }),
                    );
                } else {
                    self.show_delete_confirmation(retryable_entries);
                }
            }
            BrowserEvent::OperationCancelled {
                completed,
                failed,
                not_attempted,
                affected_locations,
            } => {
                let message = format!(
                    "{} completed, {} failed, and {} not attempted.\n\nCompleted changes were not reverted.",
                    item_count_label(*completed),
                    item_count_label(*failed),
                    item_count_label(*not_attempted),
                );
                let browser = self.browser.clone();
                let affected = affected_locations.clone();
                show_error_dialog_after_close(
                    &self.overlay,
                    "Operation cancelled",
                    &message,
                    Rc::new(move || browser.refresh_after_cancellation(&affected)),
                );
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
                    error => show_error_dialog(
                        &self.overlay,
                        "Unable to open location",
                        &error.to_string(),
                    ),
                }
            }
            BrowserEvent::ArchiveStarted { total } => {
                let browser = self.browser.clone();
                self.show_file_operation_progress(
                    *total,
                    crate::assets::icons::FILE_ARCHIVE,
                    "Working",
                    "Cancelling will not undo completed changes",
                    Rc::new(move || browser.cancel_file_operation()),
                );
            }
            BrowserEvent::ArchiveProgress { completed, total } => {
                self.update_archive_progress(*completed, *total);
            }
            BrowserEvent::ArchiveCompleted { select_name, .. } => {
                self.dismiss_file_operation_progress();
                self.pending_extract_retry.replace(None);
                if !select_name.is_empty() {
                    self.pending_select.borrow_mut().push(select_name.clone());
                }
                if let Some(dest) = self.pending_navigate.take() {
                    self.browser.navigate(dest);
                } else {
                    self.browser.reload_active();
                }
            }
            BrowserEvent::TransferCompleted => {
                if let Some(dest) = self.pending_navigate.take() {
                    self.browser.navigate(dest);
                }
            }
        }
        if Self::event_refreshes_active_path(event) {
            self.refresh_active_path_rows();
        }
        self.mode_views.borrow_mut().handle(event);
    }

    fn event_refreshes_active_path(event: &BrowserEvent) -> bool {
        matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::FocusChanged { .. }
                | BrowserEvent::SelectionSetChanged { .. }
                | BrowserEvent::EntriesInserted { .. }
                | BrowserEvent::EntriesPublished { .. }
                | BrowserEvent::EntriesSpliced { .. }
                | BrowserEvent::EntriesReplaced { .. }
        )
    }
}
