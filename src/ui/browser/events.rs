// SPDX-License-Identifier: MIT

//! Exhaustive browser event dispatch. Shared effects and column publication run before alternate
//! presentations consume the event; preserve that order when adding a feature handler.

use crate::app::BrowserEvent;
use crate::services::LocationValidationError;
use crate::ui::browser::ViewState;
use crate::ui::browser::customization;
use crate::ui::browser::location::MountStrategy;
use crate::ui::modal::show_error_dialog;
use std::rc::Rc;

mod listing;
mod operations;
mod peek;
mod reveal;
mod selection;

const MAX_BULK_REVEAL_SELECTION: usize = 64;

impl ViewState {
    pub(super) fn handle(self: &Rc<Self>, event: &BrowserEvent) {
        if event_cancels_click_rename(event) {
            self.cancel_click_rename();
        }
        if self.dispatch(event) {
            self.finish_handling(event);
        }
    }

    /// Routes one event to its handler; false when the event needs no shared follow-up.
    fn dispatch(self: &Rc<Self>, event: &BrowserEvent) -> bool {
        match event {
            BrowserEvent::BackgroundOperation { request_id, event } => {
                self.handle_background_file_operation(*request_id, event);
                return false;
            }
            BrowserEvent::SelectionSynced { .. } => return false,
            BrowserEvent::NavigationStarting { .. } => self.on_navigation_starting(),
            BrowserEvent::Reset => self.on_reset(),
            BrowserEvent::ColumnsTruncated { len, replacing } => {
                self.on_columns_truncated(*len, *replacing);
            }
            BrowserEvent::ColumnAdded { depth, location } => self.on_column_added(*depth, location),
            BrowserEvent::ColumnsRelocated { from_depth } => self.on_columns_relocated(*from_depth),
            BrowserEvent::EntriesInserted { depth, insertions } => {
                self.on_entries_inserted(*depth, insertions);
            }
            BrowserEvent::EntriesReplaced { depth, count } => {
                self.on_entries_replaced(*depth, *count);
            }
            BrowserEvent::EntriesPublished {
                depth,
                position,
                count,
            } => self.on_entries_published(*depth, *position, *count),
            BrowserEvent::FolderCountsInvalidated => crate::ui::thumbnail::refresh_metadata(),
            BrowserEvent::MetadataFilled { depth, updates } => {
                self.on_metadata_filled(*depth, updates);
            }
            BrowserEvent::SortingStarted { depth } => self.on_sorting_started(*depth),
            BrowserEvent::SortingFinished { depth } => self.on_sorting_finished(*depth),
            BrowserEvent::EntriesSpliced { depth, splices, .. } => {
                self.on_entries_spliced(*depth, splices);
            }
            BrowserEvent::ColumnRefreshing { depth } => self.on_column_refreshing(*depth),
            BrowserEvent::ColumnNotResponding {
                depth,
                not_responding,
            } => self.on_column_not_responding(*depth, *not_responding),
            BrowserEvent::ColumnReloaded { depth } => self.on_column_reloaded(*depth),
            BrowserEvent::HiddenToggled { show_hidden } => self.on_hidden_toggled(*show_hidden),
            BrowserEvent::LoadFinished { depth, truncated } => {
                self.on_load_finished(*depth, *truncated);
            }
            BrowserEvent::LoadFailed {
                depth,
                message,
                administrator,
            } => self.on_load_failed(*depth, message, administrator.is_some()),
            BrowserEvent::PeekStarted { location } => self.append_peek(location),
            BrowserEvent::PeekEntriesAdded { entries } => self.on_peek_entries_added(entries),
            BrowserEvent::PeekFinished => self.on_peek_finished(),
            BrowserEvent::PeekFailed { message } => self.on_peek_failed(message),
            BrowserEvent::PeekClosed => self.close_peek_visual(),
            BrowserEvent::SelectionSetChanged {
                depth,
                selection,
                focused,
                take_focus,
            } => self.on_selection_set_changed(&selection::SelectionChange {
                depth: *depth,
                selection,
                focused: *focused,
                take_focus: *take_focus,
            }),
            BrowserEvent::FocusChanged {
                depth,
                position,
                triggered_by_removal,
            } => self.on_focus_changed(&selection::FocusChange {
                depth: *depth,
                position: *position,
                triggered_by_removal: *triggered_by_removal,
            }),
            BrowserEvent::ColumnReloading { .. } | BrowserEvent::PreviewRequested { .. } => {}
            BrowserEvent::ExtractRequested { entry } => self.on_extract_requested(entry),
            BrowserEvent::OpenRequested { location } => self.on_open_requested(location),
            BrowserEvent::EntryCreated { location } => self.rename_created_entry(location),
            BrowserEvent::RenameCompleted { request_id } => self.on_rename_completed(*request_id),
            BrowserEvent::RenameAbandoned { request_id } => self.on_rename_abandoned(*request_id),
            BrowserEvent::RenameFailed {
                request_id,
                message,
            } => self.on_rename_failed(*request_id, message),
            BrowserEvent::TransferStarted { total, moving } => {
                self.on_transfer_started(*total, *moving);
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
                self.on_transfer_finished(moved_locations);
            }
            BrowserEvent::ItemsReported { report } => self.on_items_reported(report),
            BrowserEvent::FolderSortChosen {
                location,
                sort_key,
                sort_direction,
                name_comparison,
            } => {
                crate::ui::preferences::PreferenceManager::shared().set_folder_sort_options(
                    location,
                    (*sort_key, *sort_direction, *name_comparison),
                );
            }
            BrowserEvent::ItemRelocated { from, to, merged } => {
                customization::relocate_item_customizations(from, to, *merged);
                crate::ui::preferences::PreferenceManager::shared()
                    .relocate_folder_views(from, to, *merged);
            }
            BrowserEvent::ItemRemoved {
                location,
                trash_identity,
            } => {
                customization::forget_item_customizations(location, *trash_identity);
                crate::ui::preferences::PreferenceManager::shared()
                    .forget_folder_views_within(location, *trash_identity);
            }
            BrowserEvent::ItemRestored { location, identity } => {
                customization::restore_item_customizations(location, *identity);
                crate::ui::preferences::PreferenceManager::shared()
                    .restore_folder_views(location, *identity);
            }
            BrowserEvent::DeletionStarted { total } => self.on_deletion_started(*total),
            BrowserEvent::DeletionProgress { completed, total } => {
                self.update_item_progress(*completed, *total);
            }
            BrowserEvent::DeletionEntriesRemoved { removed } => {
                self.update_removed_entries(*removed);
            }
            BrowserEvent::DeletionFinished { succeeded } => self.on_deletion_finished(*succeeded),
            BrowserEvent::RestorationStarted { total } => self.on_restoration_started(*total),
            BrowserEvent::RestorationProgress { completed, total } => {
                self.update_item_progress(*completed, *total);
            }
            BrowserEvent::RestorationFinished { succeeded } => {
                self.on_restoration_finished(*succeeded);
            }
            BrowserEvent::OperationFailed {
                message,
                password_failure,
                report,
            } => self.on_operation_failed(
                super::transfer_report::outcome_detail(message.clone(), report),
                *password_failure,
            ),
            BrowserEvent::OperationCompletedWithErrors {
                message,
                retryable_locations,
                has_non_retryable_failures,
                report,
            } => self.on_operation_completed_with_errors(
                super::transfer_report::outcome_detail(message.clone(), report),
                retryable_locations,
                *has_non_retryable_failures,
            ),
            BrowserEvent::OperationCancelled {
                completed,
                failed,
                not_attempted,
                affected_locations,
                report,
            } => self.on_operation_cancelled(
                super::transfer_report::outcome_detail(
                    super::progress::cancelled_operation_summary(
                        *completed,
                        *failed,
                        *not_attempted,
                    ),
                    report,
                ),
                affected_locations,
            ),
            BrowserEvent::OperationRefreshRequired { depths } => {
                self.on_operation_refresh_required(depths);
            }
            BrowserEvent::NavigationRejected {
                parent_depth,
                location,
                error,
            } => self.handle_navigation_rejected(*parent_depth, location, error.clone()),
            BrowserEvent::EmptyTrashRequested => self.load_trash_summary(),
            BrowserEvent::LeftoverReviewRequested { depth } => self.show_leftover_review(*depth),
            BrowserEvent::LeftoversChanged { depth, announce } => {
                self.follow_leftovers(*depth, *announce);
            }
            BrowserEvent::LocationNavigationRejected {
                location: rejected,
                error,
            } => self.on_location_navigation_rejected(rejected, error),
            BrowserEvent::LocationRevealFailed { location } => show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Unable to select file"),
                &rust_i18n::t!(
                    "%{path} is not available in the loaded folder.",
                    path = location.display_path()
                ),
            ),
            BrowserEvent::ArchiveStarted { total } => self.on_archive_started(*total),
            BrowserEvent::ArchiveProgress { completed, total } => {
                self.update_archive_progress(*completed, *total);
            }
            BrowserEvent::ArchiveCompleted { select_name, .. } => {
                self.on_archive_completed(select_name);
            }
            BrowserEvent::TransferReveal {
                destination,
                locations,
            } => self.on_transfer_reveal(destination, locations),
            BrowserEvent::TransferCompleted => self.on_transfer_completed(),
        }
        true
    }

    /// Shared effects after the columns consumed the event, then the alternate presentations.
    fn finish_handling(self: &Rc<Self>, event: &BrowserEvent) {
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
        let results_focus = self.mode_views.borrow().take_results_focus();
        if let Some(depth) = results_focus {
            self.focus_pane_results(depth);
        }
        self.reconcile_pending_rename();
        match event {
            BrowserEvent::ColumnAdded { depth, .. } | BrowserEvent::ColumnReloaded { depth } => {
                self.note_pending_rename_refresh(*depth);
            }
            BrowserEvent::LoadFinished { depth, .. } | BrowserEvent::LoadFailed { depth, .. } => {
                self.reconcile_pending_rename_after_load(*depth);
            }
            _ => {}
        }
    }

    fn on_extract_requested(self: &Rc<Self>, entry: &crate::model::FileEntry) {
        if self.interactive {
            self.extract_entry(entry.clone());
        }
    }

    fn on_open_requested(&self, location: &crate::model::Location) {
        if !self.interactive {
            return;
        }
        let position = self
            .playback_handoff
            .borrow()
            .as_ref()
            .and_then(|handoff| handoff(location));
        super::desktop::open_location_at(location, position, &self.overlay, &self.browser);
    }

    fn on_rename_completed(self: &Rc<Self>, request_id: crate::services::OperationRequestId) {
        self.complete_pending_rename(request_id);
        self.complete_typed_placement(request_id);
        if let Some(recovered) = self.take_recovered_item() {
            self.rename_created_entry(&recovered);
        }
        self.prune_stale_search_results();
        self.finish_keyboard_refocus(
            super::file_commands::KeyboardRefocus::Rename(request_id),
            true,
        );
    }

    fn on_rename_abandoned(&self, request_id: crate::services::OperationRequestId) {
        self.abandon_pending_rename(request_id);
        if self.take_recovered_item().is_some() {
            self.cancel_new_entry();
        }
    }

    fn on_rename_failed(
        self: &Rc<Self>,
        request_id: Option<crate::services::OperationRequestId>,
        message: &str,
    ) {
        self.fail_pending_rename_from_browser(request_id);
        if self.take_recovered_item().is_some() {
            self.cancel_new_entry();
        }
        if let Some(request_id) = request_id {
            self.finish_keyboard_refocus(
                super::file_commands::KeyboardRefocus::Rename(request_id),
                false,
            );
        }
        show_error_dialog(
            &self.overlay,
            &crate::i18n::tr("Unable to rename item"),
            message,
        );
    }

    fn on_location_navigation_rejected(
        self: &Rc<Self>,
        rejected: &crate::model::Location,
        error: &LocationValidationError,
    ) {
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
            LocationValidationError::NotAuthorized => {
                self.abandon_deferred_reveal();
                self.fail_pending_connect(Some(error.message()));
            }
            error => {
                self.abandon_deferred_reveal();
                if !self.fail_pending_connect(Some(error.message())) {
                    self.show_open_failure(
                        &crate::i18n::tr("Unable to open location"),
                        rejected,
                        error,
                    );
                }
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

fn event_cancels_click_rename(event: &BrowserEvent) -> bool {
    matches!(
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
    )
}
