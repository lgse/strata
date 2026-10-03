// SPDX-License-Identifier: MIT

use crate::ui::blur::BlurBin;
use crate::ui::browser::entry::{format_file_size, item_count_label};
use crate::ui::controls::{modal_layout, progress_summary};
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod context;
mod presentation;
pub(super) use context::{BackgroundProgress, FileProgressState, file_operation_cancel};

#[cfg(test)]
mod tests;

const FILE_PROGRESS_DELAY: Duration = Duration::from_millis(350);
const TRANSFER_ITEM_TEXT_WIDTH_CHARS: i32 = 48;
const TRANSFER_RATE_TEXT_WIDTH_CHARS: i32 = 26;

#[cfg(test)]
thread_local! {
    static FILE_PROGRESS_DELAY_OVERRIDE: Cell<Option<Duration>> = const { Cell::new(None) };
}

fn file_progress_delay() -> Duration {
    #[cfg(test)]
    if let Some(delay) = FILE_PROGRESS_DELAY_OVERRIDE.with(Cell::get) {
        return delay;
    }
    FILE_PROGRESS_DELAY
}

/// Tests of fast-operation feedback must not depend on how quickly a loaded
/// CI filesystem finishes a small copy.
#[cfg(test)]
pub(super) fn set_file_progress_delay_for_test(delay: Duration) {
    FILE_PROGRESS_DELAY_OVERRIDE.with(|cell| cell.set(Some(delay)));
}

const INDETERMINATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const STALLED_CANCEL_DELAY: Duration = Duration::from_secs(8);

const IMMEDIATE_PROGRESS_ITEM_COUNT: usize = 16;

fn should_show_progress_immediately(total: usize) -> bool {
    total == 0 || total >= IMMEDIATE_PROGRESS_ITEM_COUNT
}

#[derive(Clone, Copy)]
pub(super) struct TransferProgressSnapshot {
    completed_items: usize,
    completed_files: usize,
    total_files: Option<usize>,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
}

pub(super) struct FileProgressView {
    layer: RefCell<Option<gtk::Box>>,
    icon_name: String,
    compact: Option<Rc<crate::ui::progress_dock::CompactProgress>>,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
    progress: gtk::ProgressBar,
    status: gtk::Label,
    title: gtk::Label,
    subtitle: gtk::Label,
    cancel: gtk::Button,
    cancel_action: Option<Rc<dyn Fn()>>,
    status_row: gtk::Box,
    transfer_header: gtk::Box,
    transfer_footer: gtk::Box,
    transfer_percent: gtk::Label,
    transfer_bytes: gtk::Label,
    transfer_items: gtk::Label,
    transfer_rate: gtk::Label,
    archive_activity: gtk::Spinner,
    indeterminate: Rc<Cell<bool>>,
    pulse_source: Rc<RefCell<Option<glib::SourceId>>>,
}

fn transfer_progress_status(
    completed_items: usize,
    total_items: usize,
    completed_files: usize,
    total_files: Option<usize>,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
    current_file: Option<&str>,
) -> (String, String, String, Option<f64>) {
    let items = if let Some(total_files) = total_files.filter(|total| *total > 0) {
        format!("{completed_files} of {total_files} files")
    } else if total_items > 0 {
        format!("{completed_items} of {total_items} items")
    } else {
        "Preparing items…".to_owned()
    };
    let items = match current_file.filter(|name| !name.is_empty()) {
        Some(name) => format!("{items} · {name}"),
        None => items,
    };
    let bytes = match total_bytes {
        Some(total) => format!(
            "{} / {}",
            format_file_size(transferred_bytes),
            format_file_size(total)
        ),
        None => format_file_size(transferred_bytes),
    };
    let (status, fraction) = match total_bytes {
        Some(0) if total_items > 0 => {
            let fraction = (completed_items as f64 / total_items as f64).clamp(0.0, 1.0);
            (format!("{}%", (fraction * 100.0) as usize), Some(fraction))
        }
        Some(0) => ("Preparing…".to_owned(), None),
        Some(total) => {
            let fraction = (transferred_bytes as f64 / total as f64).clamp(0.0, 1.0);
            let percentage = (fraction * 100.0) as usize;
            (
                format!(
                    "{}%",
                    if transferred_bytes > 0 {
                        percentage.max(1)
                    } else {
                        percentage
                    }
                ),
                Some(fraction),
            )
        }
        None if transferred_bytes == 0 && completed_items == 0 => ("Preparing…".to_owned(), None),
        None => ("Transferring…".to_owned(), None),
    };
    (status, bytes, items, fraction)
}

fn transfer_rate_status(rate: Option<f64>, transferred: u64, total: Option<u64>) -> String {
    let Some(rate) = rate.filter(|rate| rate.is_finite() && *rate > 0.0) else {
        return "Calculating speed…".to_owned();
    };
    let speed = format!("{}/s", format_file_size(rate as u64));
    let Some(remaining) = total.and_then(|total| total.checked_sub(transferred)) else {
        return speed;
    };
    if remaining == 0 {
        return speed;
    }
    let seconds = (remaining as f64 / rate).ceil().max(1.0) as u64;
    if seconds < 60 {
        format!("{speed} · {seconds}s left")
    } else if seconds < 3600 {
        format!("{speed} · {}m {}s left", seconds / 60, seconds % 60)
    } else {
        format!(
            "{speed} · {}h {}m left",
            seconds / 3600,
            seconds % 3600 / 60
        )
    }
}

impl FileProgressState {
    pub(super) fn show_file_operation_progress(
        self: &Rc<Self>,
        total: usize,
        icon: &str,
        title_text: &str,
        subtitle_text: &str,
        on_cancel: Rc<dyn Fn()>,
    ) {
        self.dismiss_file_operation_progress();
        self.file_operation_progress.set((0, total));
        if should_show_progress_immediately(total) {
            self.present_file_operation_progress(icon, title_text, subtitle_text, on_cancel);
            return;
        }

        let weak = Rc::downgrade(self);
        let icon = icon.to_owned();
        let title_text = title_text.to_owned();
        let subtitle_text = subtitle_text.to_owned();
        let source = glib::timeout_add_local_once(file_progress_delay(), move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.pending_file_progress.borrow_mut().take();
            state.present_file_operation_progress(&icon, &title_text, &subtitle_text, on_cancel);
        });
        self.pending_file_progress.replace(Some(source));
    }

    fn present_file_operation_progress(
        self: &Rc<Self>,
        icon: &str,
        title_text: &str,
        subtitle_text: &str,
        on_cancel: Rc<dyn Fn()>,
    ) {
        let host = if self.dock_only.get() {
            ModalHost::for_widget(&self.overlay)
        } else {
            ModalHost::blurred_for(&self.overlay)
        };
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = host
        else {
            return;
        };

        let layout = modal_layout(icon, title_text, subtitle_text, "Cancel");
        layout.content.add_css_class("compact");
        layout.close.set_visible(false);
        layout.cancel.set_visible(false);
        let status = gtk::Label::new(Some("0%"));
        status.add_css_class("modal-progress-status");
        status.set_xalign(0.0);
        let summary = progress_summary("Transferred");
        let progress = summary.progress;
        let archive_activity = gtk::Spinner::new();
        archive_activity.set_visible(false);
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status_row.append(&archive_activity);
        status_row.append(&status);
        layout.body.append(&status_row);

        let transfer_header = summary.header;
        transfer_header.set_visible(false);
        let transfer_bytes = summary.amount;
        let transfer_percent = summary.percent;
        layout.body.append(&summary.widget);

        let transfer_footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        transfer_footer.add_css_class("transfer-progress-footer");
        transfer_footer.set_visible(false);
        let transfer_items = gtk::Label::new(None);
        transfer_items.set_xalign(0.0);
        transfer_items.set_halign(gtk::Align::Start);
        transfer_items.set_hexpand(true);
        transfer_items.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        // Equal minimum and maximum text widths keep live updates from resizing the modal.
        transfer_items.set_width_chars(TRANSFER_ITEM_TEXT_WIDTH_CHARS);
        transfer_items.set_max_width_chars(TRANSFER_ITEM_TEXT_WIDTH_CHARS);
        let transfer_rate = gtk::Label::new(None);
        transfer_rate.set_xalign(1.0);
        transfer_rate.set_ellipsize(gtk::pango::EllipsizeMode::End);
        transfer_rate.set_width_chars(TRANSFER_RATE_TEXT_WIDTH_CHARS);
        transfer_rate.set_max_width_chars(TRANSFER_RATE_TEXT_WIDTH_CHARS);
        transfer_footer.append(&transfer_items);
        transfer_footer.append(&transfer_rate);
        layout.body.append(&transfer_footer);
        let title = layout.title;
        let subtitle = layout.subtitle;
        let content = layout.content;
        let cancel = layout.confirm;

        let indeterminate = Rc::new(Cell::new(false));
        let pulse_source = Rc::new(RefCell::new(None));

        let layer = if self.dock_only.get() {
            None
        } else {
            let layer = modal_layer(
                &content,
                &window_overlay,
                blurred_root.clone(),
                Some(Rc::new(|| true)),
            );
            window_overlay.add_overlay(&layer);
            Some(layer)
        };
        self.file_progress_view.replace(Some(FileProgressView {
            layer: RefCell::new(layer),
            icon_name: icon.to_owned(),
            compact: None,
            overlay: window_overlay,
            blurred_root,
            progress,
            status,
            title,
            subtitle,
            cancel: cancel.clone(),
            cancel_action: None,
            status_row,
            transfer_header,
            transfer_footer,
            transfer_percent,
            transfer_bytes,
            transfer_items,
            transfer_rate,
            archive_activity,
            indeterminate,
            pulse_source,
        }));
        let weak = Rc::downgrade(self);
        let cancel_action: Rc<dyn Fn()> = Rc::new(move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.transfer_progress.get().is_some() {
                state.request_transfer_cancel(&on_cancel);
            } else if !state.transfer_cancel_requested.replace(true) {
                if let Some(view) = state.file_progress_view.borrow().as_ref() {
                    view.cancel.set_sensitive(false);
                    view.title.set_text("Cancelling operation…");
                    view.indeterminate.set(true);
                    ensure_indeterminate_pulse(view);
                    state.sync_compact(view);
                }
                on_cancel();
            }
        });
        let click_action = cancel_action.clone();
        cancel.connect_clicked(move |_| click_action());
        if let Some(view) = self.file_progress_view.borrow_mut().as_mut() {
            view.cancel_action = Some(cancel_action.clone());
            if self.dock_only.get() {
                self.attach_dock(view);
            }
            if let Some(layer) = view.layer.borrow().as_ref() {
                presentation::install_progress_keys(layer, &cancel_action);
            }
        }
        cancel.grab_focus();
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            ensure_indeterminate_pulse(view);
        }
        if let Some(TransferProgressSnapshot {
            completed_items,
            completed_files,
            total_files,
            transferred_bytes,
            total_bytes,
        }) = self.transfer_progress.get()
        {
            self.update_transfer_progress(
                completed_items,
                completed_files,
                total_files,
                transferred_bytes,
                total_bytes,
            );
        } else if let Some((completed, total)) = self.archive_progress.get() {
            self.update_archive_progress(completed, total);
        } else if self.flushing_to_device.get() {
            self.apply_device_flush_status();
        } else {
            let (completed, total) = self.file_operation_progress.get();
            self.update_item_progress(completed, total);
        }
    }

    pub(super) fn update_transfer_progress(
        &self,
        completed_items: usize,
        completed_files: usize,
        total_files: Option<usize>,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
    ) {
        self.transfer_progress.set(Some(TransferProgressSnapshot {
            completed_items,
            completed_files,
            total_files,
            transferred_bytes,
            total_bytes,
        }));
        let now = Instant::now();
        if transferred_bytes > 0 {
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
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        let total_items = self.file_operation_progress.get().1;
        let current_file = self.transfer_current_file.borrow();
        let (status, bytes, items, fraction) = transfer_progress_status(
            completed_items,
            total_items,
            completed_files,
            total_files,
            transferred_bytes,
            total_bytes,
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
            transferred_bytes,
            total_bytes,
        ));
        if let Some(layer) = view.layer.borrow().as_ref() {
            layer.add_css_class("transfer-progress-dialog");
        }
        view.indeterminate.set(fraction.is_none());
        if let Some(fraction) = fraction {
            view.progress.set_fraction(fraction);
        }
        drop(progress_view);
        if self.flushing_to_device.get() {
            self.apply_device_flush_status();
        }
        if self.transfer_cancel_requested.get() {
            self.apply_transfer_cancel_status();
        }
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            self.sync_compact(view);
        }
    }

    fn request_transfer_cancel(self: &Rc<Self>, on_cancel: &Rc<dyn Fn()>) {
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

    fn apply_transfer_cancel_status(&self) {
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        if self.transfer_cancel_timed_out.get() {
            view.title.set_text("Device not responding");
            view.subtitle.set_text(
                "Cancellation is still pending. The device may still be writing; do not unplug it.",
            );
            view.subtitle.set_wrap(true);
            view.subtitle.set_max_width_chars(60);
            view.cancel.set_label("Cancellation requested");
            view.cancel.set_sensitive(false);
            view.transfer_rate.set_text("Waiting for device…");
        } else {
            view.title.set_text("Cancelling transfer…");
            view.subtitle
                .set_text("Waiting for the active write to stop. The device may still be writing.");
            view.subtitle.set_wrap(true);
            view.cancel.set_label("Cancellation requested");
            view.cancel.set_sensitive(false);
            view.transfer_rate.set_text("Waiting for device…");
        }
        view.indeterminate.set(true);
        ensure_indeterminate_pulse(view);
        self.sync_compact(view);
    }

    pub(super) fn show_device_flush_status(&self) {
        self.flushing_to_device.set(true);
        self.apply_device_flush_status();
    }

    fn apply_device_flush_status(&self) {
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        if self.transfer_progress.get().is_some() {
            view.transfer_percent.set_text("…");
            view.transfer_rate.set_text("Writing to device…");
        } else {
            view.status.set_text("Writing to device…");
        }
        view.indeterminate.set(true);
        view.progress.pulse();
        ensure_indeterminate_pulse(view);
        drop(progress_view);
        if self.transfer_cancel_requested.get() {
            self.apply_transfer_cancel_status();
        }
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            self.sync_compact(view);
        }
    }

    pub(super) fn update_item_progress(&self, completed: usize, total: usize) {
        self.file_operation_progress.set((completed, total));
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        let pct = if total > 0 {
            (completed as f64 / total as f64 * 100.0) as usize
        } else {
            0
        };
        view.status.set_text(&format!("{pct}%"));
        view.indeterminate.set(false);
        view.progress
            .set_fraction(completed as f64 / total.max(1) as f64);
        self.sync_compact(view);
    }

    pub(super) fn update_archive_progress(&self, completed: usize, total: usize) {
        self.file_operation_progress.set((completed, total));
        self.archive_progress.set(Some((completed, total)));
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        view.archive_activity.set_visible(true);
        view.archive_activity.start();
        if self.transfer_cancel_requested.get() {
            view.status.set_text("Stopping…");
            view.indeterminate.set(true);
        } else if completed == 0 {
            let status = if total == 0 {
                "Preparing…"
            } else if self.archive_compressing.get() {
                "Compressing…"
            } else {
                "Processing archive…"
            };
            view.status.set_text(status);
            view.indeterminate.set(true);
        } else if total == 0 {
            view.status.set_text(&format!("{completed} files"));
            view.indeterminate.set(true);
        } else {
            view.status
                .set_text(&format!("{completed} / {total} files"));
            view.indeterminate.set(false);
            view.progress.set_fraction(completed as f64 / total as f64);
        }
        ensure_indeterminate_pulse(view);
        self.sync_compact(view);
    }

    pub(super) fn complete_file_operation_progress(&self, title: &str) {
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
            compact.count.set_text(&format!("{total}/{total}"));
            compact.count.set_visible(total > 0);
            compact.completed(title);
        }
        self.dismiss_file_operation_progress();
    }

    pub(super) fn dismiss_file_operation_progress(&self) {
        self.dismiss_file_operation_progress_then(|| {});
    }

    pub(super) fn dismiss_file_operation_progress_then(
        &self,
        after_dismiss: impl FnOnce() + 'static,
    ) {
        if let Some(source) = self.pending_file_progress.take() {
            source.remove();
        }
        self.file_operation_progress.set((0, 0));
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
        if let Some(view) = self.file_progress_view.take() {
            view.indeterminate.set(false);
            view.archive_activity.stop();
            if let Some(source) = view.pulse_source.take() {
                source.remove();
            }
            if let Some(compact) = &view.compact {
                compact.remove();
            }
            if let Some(layer) = view.layer.take() {
                let after_dismiss = Rc::new(RefCell::new(Some(after_dismiss)));
                let callback = after_dismiss.clone();
                layer.connect_parent_notify(move |layer| {
                    if layer.parent().is_none()
                        && let Some(callback) = callback.borrow_mut().take()
                    {
                        callback();
                    }
                });
                dismiss_modal_layer(&layer, &view.overlay, view.blurred_root.as_ref());
            } else {
                after_dismiss();
            }
        } else {
            after_dismiss();
        }
    }

    /// The total item count isn't known upfront -- entries are deleted as they're enumerated,
    /// one bounded batch at a time -- so this pulses rather than fills to a fraction.
    pub(super) fn show_empty_trash_progress(self: &Rc<Self>, on_cancel: Rc<dyn Fn()>) {
        self.show_file_operation_progress(
            0,
            crate::assets::icons::TRASH,
            "Emptying Trash",
            "This may take a moment",
            on_cancel,
        );
        self.update_empty_trash_progress(0);
    }

    pub(super) fn update_empty_trash_progress(&self, processed: usize) {
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        view.status
            .set_text(&format!("{} deleted", item_count_label(processed)));
        view.indeterminate.set(true);
        self.sync_compact(view);
    }
}

fn ensure_indeterminate_pulse(view: &FileProgressView) {
    if view.pulse_source.borrow().is_some() {
        return;
    }
    let weak_progress = view.progress.downgrade();
    let compact_progress = view
        .compact
        .as_ref()
        .map(|compact| compact.progress.downgrade());
    let indeterminate = view.indeterminate.clone();
    let pulse_source = view.pulse_source.clone();
    let source = glib::timeout_add_local(INDETERMINATE_PROGRESS_INTERVAL, move || {
        if !indeterminate.get() {
            pulse_source.borrow_mut().take();
            return glib::ControlFlow::Break;
        }
        let Some(progress) = weak_progress.upgrade() else {
            pulse_source.borrow_mut().take();
            return glib::ControlFlow::Break;
        };
        progress.pulse();
        if let Some(compact) = compact_progress.as_ref().and_then(glib::WeakRef::upgrade) {
            compact.pulse();
        }
        glib::ControlFlow::Continue
    });
    view.pulse_source.replace(Some(source));
}
