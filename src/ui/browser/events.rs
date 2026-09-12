// SPDX-License-Identifier: MIT

//! Exhaustive browser event dispatch. Shared effects and column publication run before alternate
//! presentations consume the event; preserve that order when adding a feature handler.

use crate::app::BrowserEvent;
use crate::model::FileEntry;
use crate::services::LocationValidationError;
use crate::ui::browser::ViewState;
use crate::ui::browser::columns::{
    column_size_text, prune_missing_search_results, restore_column_cursor, scroll_column_to,
    set_column_busy, set_column_selections, set_filter_placeholder, stop_column_spinner,
    touch_source_model, update_empty_trash_sensitivity,
};
use crate::ui::browser::desktop::open_location;
use crate::ui::browser::entry::item_count_label;
use crate::ui::browser::location::MountStrategy;
use crate::ui::browser::peek::append_peek_entries;
use crate::ui::browser::trash::retryable_delete_entries;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::modal::{show_delete_error_dialog, show_error_dialog};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

impl ViewState {
    pub(super) fn handle(self: &Rc<Self>, event: &BrowserEvent) {
        match event {
            BrowserEvent::SelectionSynced { .. } => return,
            BrowserEvent::Reset => {
                self.pending_new_entry.take();
                self.pending_location_credentials.take();
                self.pending_archive_destination.take();
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
                                    .and_downcast::<crate::ui::thumbnail::ThumbnailSlot>()
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
            BrowserEvent::EntriesSpliced { depth, splices, .. } => {
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
                    let positions: Vec<_> = self
                        .browser
                        .selected_positions(*depth)
                        .into_iter()
                        .filter_map(|position| column.map.view_position(position))
                        .collect();
                    set_column_selections(column, &positions);
                    if restore_cursor
                        && let Some((focused_depth, position, _)) = self.browser.focused_item()
                        && focused_depth == *depth
                        && let Some(position) = column.map.view_position(position)
                    {
                        restore_column_cursor(column, position);
                    }
                    if count == 0 {
                        column.presentation.show_empty();
                    } else {
                        column.presentation.show_content();
                    }
                    set_column_busy(column, false);
                    update_empty_trash_sensitivity(column, count);
                }
                self.note_pending_rename_splices(*depth, splices);
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
                    let transfer_target_loaded = self
                        .pending_transfer_selection
                        .borrow()
                        .as_ref()
                        .is_some_and(|(target, _)| {
                            self.browser.location_at(*depth).as_ref() == Some(target)
                        });
                    if transfer_target_loaded
                        && self.mode_views.borrow().mode() == BrowserMode::Columns
                    {
                        self.browser.set_active_column(*depth);
                    }
                    let locations = if transfer_target_loaded {
                        self.pending_transfer_selection
                            .take()
                            .map(|(_, locations)| locations)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let names = if self.browser.active_depth() == Some(*depth)
                        && (self.mode_views.borrow().mode() != BrowserMode::Columns
                            || self.pending_archive_destination.borrow().is_none())
                    {
                        self.pending_select.take()
                    } else {
                        Vec::new()
                    };
                    let properties =
                        !names.is_empty() && self.pending_select_properties.replace(false);
                    if !names.is_empty() || !locations.is_empty() {
                        let weak = Rc::downgrade(self);
                        let depth = *depth;
                        let destination = self.browser.location_at(depth);
                        glib::idle_add_local_once(move || {
                            if let Some(state) = weak.upgrade()
                                && state.browser.location_at(depth) == destination
                            {
                                if !locations.is_empty() {
                                    state
                                        .browser
                                        .select_entries_by_location_at(depth, &locations);
                                } else if !names.is_empty() {
                                    state.browser.select_entries_by_name_at(depth, &names);
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
                                state.reveal_focused_entry();
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
                    // an in-progress rename (visible for slow network directories
                    // that stream many batches). A pending creation still needs to scroll.
                    if self.active_rename.borrow().is_none() {
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
                    let editing = self.active_rename.borrow().is_some();
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
            BrowserEvent::ExtractRequested { entry } => {
                if self.interactive {
                    self.extract_entry(entry.clone());
                }
            }
            BrowserEvent::OpenRequested { location } => {
                if self.interactive {
                    open_location(location, &self.overlay);
                }
            }
            BrowserEvent::EntryCreated { location } => {
                self.rename_created_entry(location);
            }
            BrowserEvent::RenameCompleted { request_id } => {
                self.complete_pending_rename(*request_id);
                self.prune_stale_search_results();
            }
            BrowserEvent::RenameAbandoned { request_id } => {
                self.abandon_pending_rename(*request_id);
            }
            BrowserEvent::RenameFailed {
                request_id,
                message,
            } => {
                self.fail_pending_rename_from_browser(*request_id);
                show_error_dialog(&self.overlay, "Unable to rename item", message);
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
                self.prune_stale_search_results();
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
            BrowserEvent::DeletionFinished => {
                self.clear_delete_animation();
                self.dismiss_file_operation_progress();
                self.prune_stale_search_results();
            }
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
                self.pending_new_entry.take();
                self.clear_delete_animation();
                self.dismiss_file_operation_progress();
                self.pending_archive_destination.take();
                let retry = self.pending_extract_retry.take();
                if let Some((entry, dest)) = retry
                    && extract_error_needs_password(message)
                {
                    let invalid_password = message.to_lowercase().contains("incorrect");
                    let navigate_after_extract = self.pending_navigate.take();
                    self.show_extract_password_dialog(
                        entry,
                        dest,
                        invalid_password,
                        navigate_after_extract,
                    );
                    return;
                }
                show_error_dialog(&self.overlay, "Unable to complete operation", message);
            }
            BrowserEvent::OperationCompletedWithErrors {
                message,
                retryable_locations,
                has_non_retryable_failures,
            } => {
                self.pending_archive_destination.take();
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
                self.pending_archive_destination.take();
                self.browser.refresh_after_cancellation(affected_locations);
                let message = format!(
                    "{} completed, {} failed, and {} not attempted.\n\nCompleted changes were not reverted.",
                    item_count_label(*completed),
                    item_count_label(*failed),
                    item_count_label(*not_attempted),
                );
                show_error_dialog(&self.overlay, "Operation cancelled", &message);
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
                self.pending_extract_retry.replace(None);
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
                            if !select_name.is_empty() {
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
                self.pending_archive_destination.take();
                self.pending_navigate.take();
                self.pending_select.take();
                self.pending_select_properties.set(false);
                self.pending_transfer_selection
                    .replace(Some((destination.clone(), locations.clone())));
                if self.mode_views.borrow().mode() == BrowserMode::Columns {
                    let open_depth = (0..self.columns.borrow().len()).find(|depth| {
                        self.browser.location_at(*depth).as_ref() == Some(destination)
                    });
                    let parent_depth = (0..self.columns.borrow().len())
                        .find(|depth| self.browser.location_at(*depth) == destination.parent());
                    if let Some(depth) = open_depth {
                        self.browser.set_active_column(depth);
                        self.browser.reload_active();
                    } else if let Some(parent_depth) = parent_depth {
                        self.browser.descend(parent_depth, destination.clone());
                    } else {
                        self.browser.navigate(destination.clone());
                    }
                } else if self.browser.active_location().as_ref() == Some(destination) {
                    self.browser.reload_active();
                } else {
                    self.browser.navigate(destination.clone());
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

    fn reveal_focused_entry(self: &Rc<Self>) {
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

    fn prune_stale_search_results(&self) {
        for column in self.columns.borrow().iter() {
            prune_missing_search_results(column);
        }
        self.mode_views.borrow().prune_stale_search_results();
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

fn extract_error_needs_password(message: &str) -> bool {
    // Member diagnostics quote one unescaped filename, which can itself contain backticks.
    let (prefix, suffix) = match (message.find('`'), message.rfind('`')) {
        (Some(start), Some(end)) if start < end => (&message[..start], &message[end + 1..]),
        _ => (message, ""),
    };
    [prefix, suffix].iter().any(|text| {
        let lower = text.to_lowercase();
        lower.contains("password") || lower.contains("encrypt")
    })
}

#[cfg(test)]
mod tests;
