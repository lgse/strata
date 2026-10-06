// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::glib;

use crate::{
    app::BrowserEvent,
    services::OperationRequestId,
    ui::{
        browser::{ViewState, transfer::PendingSendToCompletion},
        modal::show_error_dialog,
    },
};

use super::{FileProgressView, TransferProgressSnapshot};

type DismissWaiters = Rc<RefCell<Vec<Box<dyn FnOnce()>>>>;

pub(in crate::ui::browser) struct FileProgressState {
    pub(in crate::ui::browser) overlay: gtk::Overlay,
    pub(in crate::ui::browser) file_progress_view: RefCell<Option<FileProgressView>>,
    pub(super) file_progress_dismissing: Rc<Cell<usize>>,
    pub(super) file_progress_dismiss_waiters: DismissWaiters,
    pub(in crate::ui::browser) pending_file_progress: RefCell<Option<glib::SourceId>>,
    pub(in crate::ui::browser) file_operation_progress: Cell<(usize, usize)>,
    pub(in crate::ui::browser) archive_progress: Cell<Option<(usize, usize)>>,
    pub(in crate::ui::browser) archive_compressing: Cell<bool>,
    pub(in crate::ui::browser) deleting: Cell<bool>,
    pub(in crate::ui::browser) transfer_progress: Cell<Option<TransferProgressSnapshot>>,
    pub(super) transfer_render_source: RefCell<Option<glib::SourceId>>,
    pub(in crate::ui::browser) transfer_current_file: RefCell<Option<String>>,
    pub(in crate::ui::browser) transfer_rate_sample: Cell<Option<(std::time::Instant, u64)>>,
    pub(in crate::ui::browser) transfer_rate_bytes_per_second: Cell<Option<f64>>,
    pub(in crate::ui::browser) flushing_to_device: Cell<bool>,
    pub(in crate::ui::browser) transfer_cancel_requested: Cell<bool>,
    pub(in crate::ui::browser) transfer_cancel_timed_out: Cell<bool>,
    pub(in crate::ui::browser) transfer_cancel_timeout: RefCell<Option<glib::SourceId>>,
    pub(super) dock_only: Cell<bool>,
    pub(super) task_description: RefCell<String>,
    pub(super) destination_description: RefCell<String>,
    pub(super) destination_label: RefCell<String>,
}

impl FileProgressState {
    pub(in crate::ui::browser) fn new(overlay: &gtk::Overlay) -> Self {
        Self {
            overlay: overlay.clone(),
            file_progress_view: RefCell::new(None),
            file_progress_dismissing: Rc::new(Cell::new(0)),
            file_progress_dismiss_waiters: Rc::new(RefCell::new(Vec::new())),
            pending_file_progress: RefCell::new(None),
            file_operation_progress: Cell::new((0, 0)),
            archive_progress: Cell::new(None),
            archive_compressing: Cell::new(false),
            deleting: Cell::new(false),
            transfer_progress: Cell::new(None),
            transfer_render_source: RefCell::new(None),
            transfer_current_file: RefCell::new(None),
            transfer_rate_sample: Cell::new(None),
            transfer_rate_bytes_per_second: Cell::new(None),
            flushing_to_device: Cell::new(false),
            transfer_cancel_requested: Cell::new(false),
            transfer_cancel_timed_out: Cell::new(false),
            transfer_cancel_timeout: RefCell::new(None),
            dock_only: Cell::new(false),
            task_description: RefCell::new(String::new()),
            destination_description: RefCell::new(String::new()),
            destination_label: RefCell::new(String::new()),
        }
    }
}

impl Drop for FileProgressState {
    fn drop(&mut self) {
        self.dismiss_file_operation_progress();
    }
}

impl crate::ui::browser::BrowserView {
    pub(in crate::ui) fn dispose_file_progress(&self) {
        self.state.file_progress().dismiss_file_operation_progress();
        for background in self.state.background_file_progress.take().into_values() {
            background.progress.dismiss_file_operation_progress();
        }
    }
}

pub(in crate::ui::browser) struct BackgroundProgress {
    pub(in crate::ui::browser) progress: Rc<FileProgressState>,
    send_to: Option<PendingSendToCompletion>,
    delete_entries: Vec<crate::model::FileEntry>,
    archive_destination: Option<crate::model::Location>,
}

pub(in crate::ui::browser) fn file_operation_cancel(
    browser: Rc<crate::app::Browser>,
) -> Rc<dyn Fn()> {
    let request_id = browser.last_started_operation();
    Rc::new(move || {
        if let Some(request_id) = request_id {
            browser.cancel_operation(request_id);
        } else {
            browser.cancel_file_operation();
        }
    })
}

impl ViewState {
    pub(in crate::ui::browser) fn file_progress(&self) -> Rc<FileProgressState> {
        self.progress_state.borrow().clone()
    }

    pub(in crate::ui::browser) fn show_file_operation_progress(
        self: &Rc<Self>,
        total: usize,
        icon: &str,
        title: &str,
        subtitle: &str,
        on_cancel: Rc<dyn Fn()>,
    ) {
        let progress = if self.browser.last_started_operation().is_some() {
            let progress = Rc::new(FileProgressState::new(&self.overlay));
            self.progress_state
                .replace(progress.clone())
                .dismiss_file_operation_progress();
            progress
        } else {
            self.file_progress()
        };
        progress.archive_progress.set(None);
        progress.archive_compressing.set(false);
        progress.deleting.set(false);
        progress
            .dock_only
            .set(self.browser.backgroundable_operation().is_some());
        progress
            .destination_description
            .replace(self.browser.operation_destination_description());
        progress
            .destination_label
            .replace(self.browser.operation_destination_label());
        progress
            .task_description
            .replace(self.browser.operation_description());
        progress.show_file_operation_progress(total, icon, title, subtitle, on_cancel);
    }

    pub(in crate::ui::browser) fn dock_file_operation(
        self: &Rc<Self>,
        request_id: OperationRequestId,
    ) {
        let background = self
            .background_file_progress
            .borrow()
            .get(&request_id)
            .cloned();
        if background.is_some() {
            return;
        }
        let deleting = self.file_progress().deleting.get();
        if !self.browser.background_file_operation(request_id) {
            return;
        }
        let progress = self
            .progress_state
            .replace(Rc::new(FileProgressState::new(&self.overlay)));
        if deleting {
            self.pending_file_operation_animation.take();
        }
        if deleting
            && self.delete_dissolve_request.get().is_none()
            && self.pending_delete_dissolve.borrow().is_some()
        {
            self.delete_dissolve_request.set(Some(request_id));
        }
        let archive_destination = self.pending_archive_destination.take();
        self.pending_navigate.take();
        self.suppress_scroll_after_drop.set(false);
        self.drop_active_depths.set(None);
        self.background_file_progress.borrow_mut().insert(
            request_id,
            Rc::new(BackgroundProgress {
                progress: progress.clone(),
                send_to: self.pending_send_to_completion.take(),
                archive_destination,
                delete_entries: if deleting {
                    self.pending_delete_entries.take()
                } else {
                    Vec::new()
                },
            }),
        );
    }

    pub(in crate::ui::browser) fn handle_background_file_operation(
        self: &Rc<Self>,
        request_id: OperationRequestId,
        event: &BrowserEvent,
    ) {
        let background = self
            .background_file_progress
            .borrow()
            .get(&request_id)
            .cloned();
        let Some(background) = background else {
            return;
        };
        let progress = &background.progress;
        let finished = match event {
            BrowserEvent::TransferProgress {
                completed_items,
                completed_files,
                total_files,
                current_file,
                transferred_bytes,
                total_bytes,
            } => {
                progress.transfer_current_file.replace(current_file.clone());
                progress.update_transfer_progress(
                    *completed_items,
                    *completed_files,
                    *total_files,
                    *transferred_bytes,
                    *total_bytes,
                );
                false
            }
            BrowserEvent::FlushingToDevice => {
                progress.show_device_flush_status();
                false
            }
            BrowserEvent::ArchiveStarted { total } => {
                progress.update_archive_progress(0, *total);
                false
            }
            BrowserEvent::ArchiveProgress { completed, total } => {
                progress.update_archive_progress(*completed, *total);
                false
            }
            BrowserEvent::DeletionProgress { completed, total } => {
                progress.update_item_progress(*completed, *total);
                false
            }
            BrowserEvent::DeletionFinished { succeeded } => {
                self.prune_stale_search_results();
                if self.delete_dissolve_request.get() == Some(request_id) {
                    self.play_pending_delete_dissolve(*succeeded);
                }
                true
            }
            BrowserEvent::OperationCompletedWithErrors {
                message,
                retryable_locations,
                has_non_retryable_failures,
            } => {
                let entries = super::super::trash::retryable_delete_entries(
                    background.delete_entries.clone(),
                    retryable_locations,
                );
                if self.delete_dissolve_request.get() == Some(request_id) {
                    self.settle_pending_delete_dissolve();
                }
                if entries.is_empty() {
                    crate::ui::modal::show_partial_failure_dialog(&self.overlay, message);
                } else if *has_non_retryable_failures || self.browser.has_foreground_operation() {
                    let weak = Rc::downgrade(self);
                    crate::ui::modal::show_delete_error_dialog(
                        &self.overlay,
                        message,
                        Rc::new(move || {
                            if let Some(state) = weak.upgrade() {
                                state.retry_background_deletion(entries.clone());
                            }
                        }),
                    );
                } else {
                    self.retry_background_deletion(entries);
                }
                true
            }
            BrowserEvent::TransferFinished { .. } => false,
            BrowserEvent::TransferCompleted => {
                if progress.file_progress_view.borrow().is_none()
                    && let Some(completion) = &background.send_to
                {
                    self.show_send_to_success(&completion.device_name, completion.item_count);
                }
                true
            }
            BrowserEvent::TransferReveal { .. } => {
                self.handle(event);
                false
            }
            BrowserEvent::ArchiveCompleted { select_name, .. } => {
                if !select_name.is_empty() {
                    self.pending_archive_destination
                        .replace(background.archive_destination.clone());
                    self.handle(event);
                }
                true
            }
            BrowserEvent::OperationFailed { message, .. } => {
                if self.delete_dissolve_request.get() == Some(request_id) {
                    self.settle_pending_delete_dissolve();
                }
                show_error_dialog(
                    &self.overlay,
                    &crate::i18n::tr("Unable to complete operation"),
                    message,
                );
                true
            }
            BrowserEvent::OperationCancelled {
                completed,
                failed,
                not_attempted,
                affected_locations,
            } => {
                if self.delete_dissolve_request.get() == Some(request_id) {
                    self.settle_pending_delete_dissolve();
                }
                self.browser.refresh_after_cancellation(affected_locations);
                show_error_dialog(
                    &self.overlay,
                    &crate::i18n::tr("Operation cancelled"),
                    &super::cancelled_operation_summary(*completed, *failed, *not_attempted),
                );
                true
            }
            _ => false,
        };
        if finished {
            match event {
                BrowserEvent::TransferCompleted => {
                    progress.complete_file_operation_progress(&crate::i18n::tr("Copy complete"))
                }
                BrowserEvent::ArchiveCompleted { .. } => progress
                    .complete_file_operation_progress(&crate::i18n::tr("Compression complete")),
                BrowserEvent::DeletionFinished { succeeded: true } => {
                    progress.complete_file_operation_progress(&crate::i18n::tr("Deletion complete"))
                }
                _ => progress.dismiss_file_operation_progress(),
            }
            self.background_file_progress
                .borrow_mut()
                .remove(&request_id);
        }
    }

    fn retry_background_deletion(self: &Rc<Self>, entries: Vec<crate::model::FileEntry>) {
        let visible = self.file_progress().file_progress_view.borrow().is_some();
        if visible && let Some(id) = self.browser.backgroundable_operation() {
            self.dock_file_operation(id);
        }
        if self.browser.has_foreground_operation() {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Another operation is active"),
                &crate::i18n::tr(
                    "Finish or minimize that operation before retrying deletion. The failed items have not been deleted.",
                ),
            );
            return;
        }
        self.show_trash_unavailable_confirmation(entries);
    }

    pub(in crate::ui::browser) fn update_transfer_progress(
        &self,
        completed_items: usize,
        completed_files: usize,
        total_files: Option<usize>,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
    ) {
        self.file_progress().update_transfer_progress(
            completed_items,
            completed_files,
            total_files,
            transferred_bytes,
            total_bytes,
        );
    }
    pub(in crate::ui::browser) fn update_item_progress(&self, completed: usize, total: usize) {
        self.file_progress().update_item_progress(completed, total);
    }
    pub(in crate::ui::browser) fn update_archive_progress(&self, completed: usize, total: usize) {
        self.file_progress()
            .update_archive_progress(completed, total);
    }
    pub(in crate::ui::browser) fn show_device_flush_status(&self) {
        self.file_progress().show_device_flush_status();
    }
    pub(in crate::ui::browser) fn dismiss_file_operation_progress(&self) {
        self.file_progress().dismiss_file_operation_progress();
    }
    pub(in crate::ui::browser) fn dismiss_file_operation_progress_then(
        &self,
        callback: impl FnOnce() + 'static,
    ) {
        self.file_progress()
            .dismiss_file_operation_progress_then(callback);
    }
    pub(in crate::ui::browser) fn show_empty_trash_progress(
        self: &Rc<Self>,
        on_cancel: Rc<dyn Fn()>,
    ) {
        let progress = self.file_progress();
        progress.dock_only.set(false);
        progress.task_description.borrow_mut().clear();
        progress.show_empty_trash_progress(on_cancel);
    }
    pub(in crate::ui::browser) fn update_empty_trash_progress(&self, completed: usize) {
        self.file_progress().update_empty_trash_progress(completed);
    }
}
