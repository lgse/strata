// SPDX-License-Identifier: MIT

//! File operation lifecycle: transfers, deletions, restorations, archives and their outcomes.

use crate::model::{FileEntry, Location};
use crate::services::{PasswordFailure, TransferReport};
use crate::ui::browser::ViewState;
use crate::ui::browser::archive::extract_password_retry;
use crate::ui::browser::transfer::FinishedSendToCompletion;
use crate::ui::browser::trash::retryable_delete_entries;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::modal::{show_delete_error_dialog, show_error_dialog};
use gtk::glib;
use std::collections::HashSet;
use std::rc::Rc;

type Dissolve = (usize, crate::ui::browser::dissolve_delete::PreparedDissolve);

impl ViewState {
    fn dock_backgroundable_operation(self: &Rc<Self>) {
        if let Some(id) = self.browser.backgroundable_operation() {
            self.dock_file_operation(id);
        }
    }

    /// Drops the drop-scroll suppression and the reveal, send-to and archive follow-ups
    /// an interrupted operation can no longer complete.
    fn abandon_operation_follow_ups(&self) {
        self.suppress_scroll_after_drop.set(false);
        self.drop_active_depths.set(None);
        self.pending_archive_destination.take();
        self.pending_send_to_completion.take();
        self.finished_send_to_completion.take();
    }

    pub(super) fn on_transfer_started(self: &Rc<Self>, total: usize, moving: bool) {
        let browser = self.browser.clone();
        self.show_transfer_progress(
            total,
            moving,
            crate::ui::browser::progress::file_operation_cancel(browser),
        );
        self.update_transfer_progress(0, 0, None, 0, None);
        self.dock_backgroundable_operation();
    }

    pub(super) fn on_transfer_finished(self: &Rc<Self>, moved_locations: &[Location]) {
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

    pub(super) fn on_items_reported(self: &Rc<Self>, report: &TransferReport) {
        let report = report.clone();
        let weak = Rc::downgrade(self);
        self.dismiss_file_operation_progress_then(move || {
            if let Some(state) = weak.upgrade() {
                state.show_transfer_report(&report);
            }
        });
    }

    pub(super) fn on_deletion_started(self: &Rc<Self>, total: usize) {
        let browser = self.browser.clone();
        self.show_file_operation_progress(
            total,
            crate::assets::icons::TRASH,
            "Deleting items",
            "Cancelling will not undo completed changes",
            crate::ui::browser::progress::file_operation_cancel(browser),
        );
        self.file_progress().deleting.set(true);
        self.dock_backgroundable_operation();
    }

    pub(super) fn on_deletion_finished(self: &Rc<Self>, succeeded: bool) {
        let dissolve = self.pending_delete_dissolve.take();
        if let Some((depth, _)) = dissolve.as_ref() {
            self.deferred_delete_empty_depth.set(Some(*depth));
        }
        let has_animation =
            dissolve.is_some() || self.pending_file_operation_animation.borrow().is_some();
        if has_animation {
            let weak = Rc::downgrade(self);
            self.dismiss_file_operation_progress_then(move || {
                glib::idle_add_local_once(move || {
                    if let Some(state) = weak.upgrade() {
                        state.settle_deletion_animation(succeeded, dissolve);
                    }
                });
            });
        } else {
            self.dismiss_file_operation_progress();
        }
        self.prune_stale_search_results();
    }

    fn settle_deletion_animation(self: &Rc<Self>, succeeded: bool, dissolve: Option<Dissolve>) {
        if succeeded {
            self.play_pending_file_operation_animation();
            self.play_delete_dissolve(dissolve);
        } else {
            self.clear_delete_animation();
            if let Some((depth, _)) = dissolve {
                self.finish_delete_animation(depth);
            }
        }
    }

    fn play_delete_dissolve(self: &Rc<Self>, dissolve: Option<Dissolve>) {
        let Some((depth, dissolve)) = dissolve else {
            return;
        };
        let weak = Rc::downgrade(self);
        dissolve.play(move || {
            if let Some(state) = weak.upgrade() {
                state.finish_delete_animation(depth);
            }
        });
    }

    pub(super) fn on_restoration_started(self: &Rc<Self>, total: usize) {
        let browser = self.browser.clone();
        self.show_file_operation_progress(
            total,
            crate::assets::icons::FOLDER,
            "Restoring items",
            "Cancelling will not undo completed changes",
            crate::ui::browser::progress::file_operation_cancel(browser),
        );
    }

    pub(super) fn on_restoration_finished(self: &Rc<Self>, succeeded: bool) {
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

    /// `message` is the outcome with the report's detail already appended.
    pub(super) fn on_operation_failed(
        self: &Rc<Self>,
        message: String,
        password_failure: Option<PasswordFailure>,
    ) {
        self.abandon_operation_follow_ups();
        self.pending_new_entry.take();
        let retry = self.pending_extract_retry.take();
        let navigate_after_extract = self.pending_navigate.take();
        let weak = Rc::downgrade(self);
        self.dismiss_file_operation_progress_then(move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.clear_delete_animation();
            if let Some((entry, destination)) = retry
                && let Some(invalid_password) = extract_password_retry(password_failure)
            {
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

    /// `message` is the outcome with the report's detail already appended.
    pub(super) fn on_operation_completed_with_errors(
        self: &Rc<Self>,
        message: String,
        retryable_locations: &[Location],
        has_non_retryable_failures: bool,
    ) {
        self.abandon_operation_follow_ups();
        let retryable_entries =
            retryable_delete_entries(self.pending_delete_entries.take(), retryable_locations);
        let weak = Rc::downgrade(self);
        self.dismiss_file_operation_progress_then(move || {
            if let Some(state) = weak.upgrade() {
                state.report_partial_failure(
                    &message,
                    retryable_entries,
                    has_non_retryable_failures,
                );
            }
        });
    }

    fn report_partial_failure(
        self: &Rc<Self>,
        message: &str,
        retryable_entries: Vec<FileEntry>,
        has_non_retryable_failures: bool,
    ) {
        if retryable_entries.is_empty() {
            crate::ui::modal::show_partial_failure_dialog(&self.overlay, message);
        } else if has_non_retryable_failures {
            let weak_state = Rc::downgrade(self);
            show_delete_error_dialog(
                &self.overlay,
                message,
                &crate::ui::browser::trash::delete_retry_label(&retryable_entries),
                Rc::new(move || {
                    if let Some(state) = weak_state.upgrade() {
                        state.show_trash_unavailable_confirmation(retryable_entries.clone());
                    }
                }),
            );
        } else {
            self.show_trash_unavailable_confirmation(retryable_entries);
        }
    }

    /// `message` is the cancellation summary with the report's detail already appended.
    pub(super) fn on_operation_cancelled(
        self: &Rc<Self>,
        message: String,
        affected_locations: &HashSet<Location>,
    ) {
        self.abandon_operation_follow_ups();
        self.pending_navigate.take();
        let affected_locations = affected_locations.clone();
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

    pub(super) fn on_operation_refresh_required(&self, depths: &[usize]) {
        let browser = self.browser.clone();
        let depths = depths.to_vec();
        self.dismiss_file_operation_progress_then(move || {
            browser.refresh_operation_columns(&depths);
        });
    }

    pub(super) fn on_archive_started(self: &Rc<Self>, total: usize) {
        let browser = self.browser.clone();
        let compressing = self.browser.backgroundable_operation().is_some();
        self.show_file_operation_progress(
            total,
            crate::assets::icons::FILE_ARCHIVE,
            if compressing {
                "Compressing items"
            } else {
                "Processing archive…"
            },
            "Cancelling will not undo completed changes",
            crate::ui::browser::progress::file_operation_cancel(browser),
        );
        self.file_progress()
            .archive_compressing
            .set(self.browser.backgroundable_operation().is_some());
        self.update_archive_progress(0, total);
        self.dock_backgroundable_operation();
    }

    pub(super) fn on_archive_completed(self: &Rc<Self>, select_name: &str) {
        self.pending_extract_retry.replace(None);
        let extract_destination = self.extract_destination.take();
        if select_name.is_empty() {
            self.pending_archive_destination.take();
        }
        if let Some(destination) = self.pending_navigate.take() {
            self.navigate_to_archive_result(destination, select_name.to_owned());
            return;
        }
        let pending_destination = (!select_name.is_empty())
            .then(|| self.pending_archive_destination.borrow().clone())
            .flatten();
        match pending_destination {
            Some(destination) => self.reveal_archive_result(destination, select_name.to_owned()),
            None => self.show_archive_result_later(extract_destination, select_name.to_owned()),
        }
    }

    fn navigate_to_archive_result(self: &Rc<Self>, destination: Location, select_name: String) {
        let weak = Rc::downgrade(self);
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
    }

    fn reveal_archive_result(self: &Rc<Self>, destination: Location, select_name: String) {
        let weak = Rc::downgrade(self);
        self.dismiss_file_operation_progress_then(move || {
            glib::idle_add_local_once(move || {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if state.pending_archive_destination.borrow().as_ref() != Some(&destination) {
                    return;
                }
                state.pending_select.borrow_mut().push(select_name);
                match state.archive_destination_depth(&destination) {
                    Some(depth) => state.reveal_pending_archive_at(depth),
                    None => state.reload_archive_destination(destination),
                }
            });
        });
    }

    /// The open depth showing `destination`: any column, or only the active pane elsewhere.
    fn archive_destination_depth(&self, destination: &Location) -> Option<usize> {
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            (0..self.columns.borrow().len())
                .find(|depth| self.browser.location_at(*depth).as_ref() == Some(destination))
        } else {
            self.browser
                .active_depth()
                .filter(|depth| self.browser.location_at(*depth).as_ref() == Some(destination))
        }
    }

    fn show_archive_result_later(
        self: &Rc<Self>,
        extract_destination: Option<Location>,
        select_name: String,
    ) {
        let weak = Rc::downgrade(self);
        let navigation_generation = self.browser.navigation_generation();
        self.dismiss_file_operation_progress_then(move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let current = state.browser.navigation_generation() == navigation_generation
                && !select_name.is_empty();
            if current {
                state.show_archive_result(extract_destination, select_name);
            }
        });
    }

    pub(super) fn on_transfer_reveal(
        self: &Rc<Self>,
        destination: &Location,
        locations: &[Location],
    ) {
        let weak = Rc::downgrade(self);
        let destination = destination.clone();
        let locations = locations.to_vec();
        self.dismiss_file_operation_progress_then(move || {
            if let Some(state) = weak.upgrade() {
                state.apply_transfer_reveal(destination, locations);
            }
        });
    }

    pub(super) fn on_transfer_completed(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.dismiss_file_operation_progress_then(move || {
            if let Some(state) = weak.upgrade() {
                state.complete_transfer_ui();
            }
        });
    }

    pub(in crate::ui::browser) fn play_pending_delete_dissolve(self: &Rc<Self>, succeeded: bool) {
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

    pub(in crate::ui::browser) fn settle_pending_delete_dissolve(&self) {
        self.delete_dissolve_request.set(None);
        let Some((depth, _)) = self.pending_delete_dissolve.take() else {
            return;
        };
        self.deferred_delete_empty_depth.set(Some(depth));
        self.finish_delete_animation(depth);
    }

    pub(super) fn finish_delete_animation(&self, depth: usize) {
        if self.deferred_delete_empty_depth.get() != Some(depth) {
            return;
        }
        self.deferred_delete_empty_depth.set(None);
        if self.delete_animation_defers_empty_state(depth) {
            return;
        }
        let column_empty =
            self.columns.borrow().get(depth).is_some_and(|column| {
                column.entry_count.get() == 0 && !column.spinner.is_spinning()
            });
        if column_empty && let Some(column) = self.columns.borrow().get(depth) {
            column.presentation.show_empty_if_ready();
        }
        self.mode_views.borrow().show_empty_if_empty(depth);
    }
}
