// SPDX-License-Identifier: MIT

use super::{FileProgressState, FileProgressView};
use crate::ui::modal::dismiss_modal_layer;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type DismissWaiters = Rc<RefCell<Vec<Box<dyn FnOnce()>>>>;

fn run_dismiss_waiters(waiters: &DismissWaiters) {
    let callbacks = waiters.take();
    for callback in callbacks {
        callback();
    }
}

impl FileProgressState {
    pub(in crate::ui::browser) fn complete_file_operation_progress(&self, title: &str) {
        let compact = self
            .file_progress_view
            .borrow_mut()
            .as_mut()
            .and_then(|view| view.compact.take());
        if let Some(compact) = compact {
            let file = self.transfer_current_file.borrow();
            let description = self.task_description.borrow();
            compact.info.set_text(
                file.as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&description),
            );
            let total = self
                .transfer_progress
                .get()
                .and_then(|snapshot| snapshot.total_files)
                .unwrap_or(self.file_operation_progress.get().1);
            let total_text = crate::i18n::integer(total as u64);
            compact
                .count
                .set_text(&format!("{total_text}/{total_text}"));
            compact.count.set_visible(total > 0);
            compact.completed(title);
        }
        self.dismiss_file_operation_progress();
    }

    pub(in crate::ui::browser) fn dismiss_file_operation_progress(&self) {
        self.dismiss_file_operation_progress_then(|| {});
    }

    pub(in crate::ui::browser) fn dismiss_file_operation_progress_then(
        &self,
        after_dismiss: impl FnOnce() + 'static,
    ) {
        self.reset_operation_tracking();
        self.file_progress_dismiss_waiters
            .borrow_mut()
            .push(Box::new(after_dismiss));
        if let Some(view) = self.file_progress_view.take() {
            self.tear_down_view(&view);
        }
        if self.file_progress_dismissing.get() == 0 {
            run_dismiss_waiters(&self.file_progress_dismiss_waiters);
        }
    }

    fn reset_operation_tracking(&self) {
        if let Some(source) = self.pending_file_progress.take() {
            source.remove();
        }
        self.on_present.take();
        self.file_operation_progress.set((0, 0));
        if let Some(source) = self.transfer_render_source.take() {
            source.remove();
        }
        self.archive_progress.set(None);
        if let Some(source) = self.transfer_cancel_timeout.take() {
            source.remove();
        }
        self.transfer_cancel_requested.set(false);
        self.transfer_cancel_timed_out.set(false);
        self.transfer_progress.set(None);
        self.transfer_current_file.take();
        self.transfer_rate_sample.set(None);
        self.transfer_rate_bytes_per_second.set(None);
        self.flushing_to_device.set(false);
    }

    fn tear_down_view(&self, view: &FileProgressView) {
        view.indeterminate.set(false);
        view.archive_activity.stop();
        if let Some(source) = view.pulse_source.take() {
            source.remove();
        }
        if let Some(compact) = &view.compact {
            compact.remove();
        }
        if let Some(layer) = view.layer.take() {
            self.dismiss_view_layer(view, &layer);
        }
    }

    /// Waiters run once the last dismissing layer has left the overlay.
    fn dismiss_view_layer(&self, view: &FileProgressView, layer: &gtk::Box) {
        let pending = self.file_progress_dismissing.clone();
        let waiters = self.file_progress_dismiss_waiters.clone();
        pending.set(pending.get() + 1);
        let finished = Cell::new(false);
        layer.connect_parent_notify(move |layer| {
            if layer.parent().is_none() && !finished.replace(true) {
                pending.set(pending.get() - 1);
                if pending.get() == 0 {
                    run_dismiss_waiters(&waiters);
                }
            }
        });
        dismiss_modal_layer(layer, &view.overlay, view.blurred_root.as_ref());
    }
}
