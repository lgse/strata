// SPDX-License-Identifier: MIT

use super::{
    FileProgressState, FileProgressView, PROGRESS_THROTTLE_INTERVAL, STALLED_CANCEL_DELAY,
    TransferProgressSnapshot, transfer_progress_status, transfer_rate_status,
};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::{Duration, Instant};

impl TransferProgressSnapshot {
    fn is_terminal(&self, total_items: usize) -> bool {
        (total_items > 0 && self.completed_items >= total_items)
            || self
                .total_files
                .is_some_and(|total| total > 0 && self.completed_files >= total)
            || self
                .total_bytes
                .is_some_and(|total| total > 0 && self.transferred_bytes >= total)
    }

    fn is_initial(&self) -> bool {
        self.completed_items == 0 && self.completed_files == 0 && self.transferred_bytes == 0
    }
}

impl FileProgressState {
    pub(in crate::ui::browser) fn update_transfer_progress(
        self: &Rc<Self>,
        completed_items: usize,
        completed_files: usize,
        total_files: Option<usize>,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
    ) {
        let snapshot = TransferProgressSnapshot {
            completed_items,
            completed_files,
            total_files,
            transferred_bytes,
            total_bytes,
        };
        self.transfer_progress.set(Some(snapshot));
        if transferred_bytes > 0 {
            self.sample_transfer_rate(transferred_bytes);
        }
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        let total_items = self.file_operation_progress.get().1;
        let is_initial = snapshot.is_initial();
        let now = Instant::now();
        if !snapshot.is_terminal(total_items) && !is_initial && rendered_recently(view, now) {
            self.schedule_throttled_render();
            return;
        }
        if !is_initial {
            view.last_transfer_render.set(Some(now));
        }
        if let Some(source) = self.transfer_render_source.take() {
            source.remove();
        }
        self.render_transfer_progress(view, &snapshot, total_items);
        drop(progress_view);
        if self.flushing_to_device.get() {
            self.apply_device_flush_status();
        }
        self.sync_after_transfer_update();
    }

    /// Smooths the measured rate so the time-left estimate does not jump between samples.
    fn sample_transfer_rate(&self, transferred_bytes: u64) {
        let now = Instant::now();
        match self.transfer_rate_sample.get() {
            Some((last, last_bytes))
                if transferred_bytes > last_bytes
                    && now.duration_since(last) >= Duration::from_millis(250) =>
            {
                let measured = (transferred_bytes - last_bytes) as f64
                    / now.duration_since(last).as_secs_f64();
                let rate = self
                    .transfer_rate_bytes_per_second
                    .get()
                    .map_or(measured, |previous| previous * 0.6 + measured * 0.4);
                self.transfer_rate_bytes_per_second.set(Some(rate));
                self.transfer_rate_sample
                    .set(Some((now, transferred_bytes)));
            }
            None => self
                .transfer_rate_sample
                .set(Some((now, transferred_bytes))),
            _ => {}
        }
    }

    fn schedule_throttled_render(self: &Rc<Self>) {
        if self.transfer_render_source.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(PROGRESS_THROTTLE_INTERVAL, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.transfer_render_source.take();
            if let Some(snapshot) = state.transfer_progress.get() {
                state.update_transfer_progress(
                    snapshot.completed_items,
                    snapshot.completed_files,
                    snapshot.total_files,
                    snapshot.transferred_bytes,
                    snapshot.total_bytes,
                );
            }
        });
        self.transfer_render_source.replace(Some(source));
    }

    fn render_transfer_progress(
        &self,
        view: &FileProgressView,
        snapshot: &TransferProgressSnapshot,
        total_items: usize,
    ) {
        let current_file = self.transfer_current_file.borrow();
        let (status, bytes, items, fraction) = transfer_progress_status(
            snapshot.completed_items,
            total_items,
            snapshot.completed_files,
            snapshot.total_files,
            snapshot.transferred_bytes,
            snapshot.total_bytes,
            current_file.as_deref(),
        );
        view.status_row.set_visible(false);
        view.transfer_header.set_visible(true);
        view.transfer_footer.set_visible(true);
        view.transfer_percent.set_text(&status);
        view.transfer_bytes.set_text(&bytes);
        view.transfer_items.set_text(&items);
        view.transfer_rate.set_text(&transfer_rate_status(
            self.transfer_rate_bytes_per_second.get(),
            snapshot.transferred_bytes,
            snapshot.total_bytes,
        ));
        if let Some(layer) = view.layer.borrow().as_ref() {
            layer.add_css_class("transfer-progress-dialog");
        }
        view.indeterminate.set(fraction.is_none());
        if let Some(fraction) = fraction {
            view.progress.set_fraction(fraction);
        }
    }

    fn sync_after_transfer_update(&self) {
        if self.transfer_cancel_requested.get() {
            self.apply_transfer_cancel_status();
        }
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            self.sync_compact(view);
        }
    }

    pub(super) fn request_transfer_cancel(self: &Rc<Self>, on_cancel: &Rc<dyn Fn()>) {
        if self.transfer_cancel_requested.replace(true) {
            return;
        }
        self.apply_transfer_cancel_status();
        let weak = Rc::downgrade(self);
        let timeout = glib::timeout_add_local_once(STALLED_CANCEL_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.transfer_cancel_timeout.borrow_mut().take();
            if state.transfer_cancel_requested.get() {
                state.transfer_cancel_timed_out.set(true);
                state.apply_transfer_cancel_status();
            }
        });
        self.transfer_cancel_timeout.replace(Some(timeout));
        on_cancel();
    }

    pub(super) fn apply_transfer_cancel_status(&self) {
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        if self.transfer_cancel_timed_out.get() {
            view.title
                .set_text(&crate::i18n::tr("Device not responding"));
            view.subtitle.set_text(&crate::i18n::tr(
                "Cancellation is still pending. The device may still be writing; do not unplug it.",
            ));
            view.subtitle.set_wrap(true);
            view.subtitle.set_max_width_chars(60);
        } else {
            view.title
                .set_text(&crate::i18n::tr("Cancelling transfer…"));
            view.subtitle.set_text(&crate::i18n::tr(
                "Waiting for the active write to stop. The device may still be writing.",
            ));
            view.subtitle.set_wrap(true);
        }
        view.cancel
            .set_label(&crate::i18n::tr("Cancellation requested"));
        view.cancel.set_sensitive(false);
        view.transfer_rate
            .set_text(&crate::i18n::tr("Waiting for device…"));
        view.indeterminate.set(true);
        view.ensure_indeterminate_pulse();
        self.sync_compact(view);
    }

    pub(in crate::ui::browser) fn show_device_flush_status(&self) {
        self.flushing_to_device.set(true);
        self.apply_device_flush_status();
    }

    pub(super) fn apply_device_flush_status(&self) {
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        if self.transfer_progress.get().is_some() {
            view.transfer_percent.set_text("…");
            view.transfer_rate
                .set_text(&crate::i18n::tr("Writing to device…"));
        } else {
            view.status.set_text(&crate::i18n::tr("Writing to device…"));
        }
        view.indeterminate.set(true);
        view.progress.pulse();
        view.ensure_indeterminate_pulse();
        drop(progress_view);
        self.sync_after_transfer_update();
    }
}

fn rendered_recently(view: &FileProgressView, now: Instant) -> bool {
    view.last_transfer_render
        .get()
        .is_some_and(|last| now.duration_since(last) < PROGRESS_THROTTLE_INTERVAL)
}
