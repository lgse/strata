// SPDX-License-Identifier: MIT

use crate::ui::blur::BlurBin;
use crate::ui::browser::ViewState;
use crate::ui::browser::entry::{format_file_size, item_count_label};
use crate::ui::controls::modal_layout;
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const FILE_PROGRESS_DELAY: Duration = Duration::from_millis(350);

const INDETERMINATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

const IMMEDIATE_PROGRESS_ITEM_COUNT: usize = 16;

fn should_show_progress_immediately(total: usize) -> bool {
    total == 0 || total >= IMMEDIATE_PROGRESS_ITEM_COUNT
}

pub(super) struct FileProgressView {
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
    progress: gtk::ProgressBar,
    status: gtk::Label,
    archive_activity: gtk::Spinner,
    indeterminate: Rc<Cell<bool>>,
    pulse_source: Rc<RefCell<Option<glib::SourceId>>>,
}

fn transfer_progress_status(
    completed_items: usize,
    total_items: usize,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
) -> (String, Option<f64>) {
    match total_bytes {
        Some(0) if total_items > 0 => {
            let fraction = (completed_items as f64 / total_items as f64).clamp(0.0, 1.0);
            let percentage = (fraction * 100.0) as usize;
            (format!("{percentage}%"), Some(fraction))
        }
        Some(0) => ("Preparing…".to_owned(), None),
        Some(total) => {
            let fraction = (transferred_bytes as f64 / total as f64).clamp(0.0, 1.0);
            let percentage = (fraction * 100.0) as usize;
            let percentage = if transferred_bytes > 0 {
                percentage.max(1)
            } else {
                percentage
            };
            (format!("{percentage}%"), Some(fraction))
        }
        None if transferred_bytes == 0 && completed_items == 0 => ("Preparing…".to_owned(), None),
        None if transferred_bytes == 0 => (
            format!(
                "{completed_items} {} copied",
                if completed_items == 1 {
                    "item"
                } else {
                    "items"
                }
            ),
            None,
        ),
        None => (
            format!("{} copied", format_file_size(transferred_bytes)),
            None,
        ),
    }
}

impl ViewState {
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
        let source = glib::timeout_add_local_once(FILE_PROGRESS_DELAY, move || {
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
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
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
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("modal-progress");
        progress.set_fraction(0.0);
        let archive_activity = gtk::Spinner::new();
        archive_activity.set_visible(false);
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status_row.append(&archive_activity);
        status_row.append(&status);
        layout.body.append(&status_row);
        layout.body.append(&progress);
        let content = layout.content;
        let cancel = layout.confirm;

        let indeterminate = Rc::new(Cell::new(false));
        let pulse_source = Rc::new(RefCell::new(None));

        let layer = modal_layer(
            &content,
            &window_overlay,
            blurred_root.clone(),
            Some(Rc::new(|| true)),
        );
        window_overlay.add_overlay(&layer);
        self.file_progress_view.replace(Some(FileProgressView {
            layer,
            overlay: window_overlay,
            blurred_root,
            progress,
            status,
            archive_activity,
            indeterminate,
            pulse_source,
        }));
        let cancel_action = on_cancel.clone();
        cancel.connect_clicked(move |_| cancel_action());
        let escape = gtk::EventControllerKey::new();
        let escape_action = on_cancel;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                escape_action();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        if let Some(progress) = self.file_progress_view.borrow().as_ref() {
            progress.layer.add_controller(escape);
        }
        cancel.grab_focus();
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            ensure_indeterminate_pulse(&view.progress, &view.indeterminate, &view.pulse_source);
        }
        if let Some((completed_items, transferred_bytes, total_bytes)) =
            self.transfer_progress.get()
        {
            self.update_transfer_progress(completed_items, transferred_bytes, total_bytes);
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
        transferred_bytes: u64,
        total_bytes: Option<u64>,
    ) {
        self.transfer_progress
            .set(Some((completed_items, transferred_bytes, total_bytes)));
        if self.flushing_to_device.get() {
            self.apply_device_flush_status();
            return;
        }
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        let total_items = self.file_operation_progress.get().1;
        let (status, fraction) =
            transfer_progress_status(completed_items, total_items, transferred_bytes, total_bytes);
        view.status.set_text(&status);
        view.indeterminate.set(fraction.is_none());
        if let Some(fraction) = fraction {
            view.progress.set_fraction(fraction);
        }
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
        view.status.set_text("Writing to device…");
        view.indeterminate.set(true);
        view.progress.pulse();
        ensure_indeterminate_pulse(&view.progress, &view.indeterminate, &view.pulse_source);
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
    }

    pub(super) fn update_archive_progress(&self, completed: usize, total: usize) {
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        view.archive_activity.set_visible(true);
        view.archive_activity.start();
        if completed == 0 {
            view.status.set_text("Preparing…");
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
        self.transfer_progress.set(None);
        self.flushing_to_device.set(false);
        if let Some(view) = self.file_progress_view.take() {
            view.indeterminate.set(false);
            view.archive_activity.stop();
            if let Some(source) = view.pulse_source.take() {
                source.remove();
            }
            let after_dismiss = Rc::new(RefCell::new(Some(after_dismiss)));
            let callback = after_dismiss.clone();
            view.layer.connect_parent_notify(move |layer| {
                if layer.parent().is_none()
                    && let Some(callback) = callback.borrow_mut().take()
                {
                    callback();
                }
            });
            dismiss_modal_layer(&view.layer, &view.overlay, view.blurred_root.as_ref());
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
    }

    /// Floating corner chip for a pasted-URL download. Unlike the operation
    /// modal it never blocks the view; only its own button cancels the fetch.
    pub(super) fn show_download_progress(&self, url: &str, on_cancel: Rc<dyn Fn()>) {
        self.dismiss_download_progress();
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        chip.add_css_class("download-chip");
        chip.set_halign(gtk::Align::End);
        chip.set_valign(gtk::Align::End);
        chip.set_margin_end(16);
        chip.set_margin_bottom(16);
        chip.set_tooltip_text(Some(url));
        chip.append(&crate::assets::primary_icon(
            crate::assets::icons::DOWNLOADS,
            16,
        ));

        let name = gtk::Label::new(Some(
            &crate::services::remote_file_name(url).unwrap_or_else(|| url.to_owned()),
        ));
        name.add_css_class("download-chip-name");
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        name.set_max_width_chars(28);
        chip.append(&name);

        let progress = gtk::ProgressBar::new();
        progress.add_css_class("job-progress");
        progress.set_size_request(140, -1);
        progress.set_valign(gtk::Align::Center);
        chip.append(&progress);

        let status = gtk::Label::new(Some("Connecting…"));
        status.add_css_class("download-chip-status");
        chip.append(&status);

        let cancel = gtk::Button::new();
        cancel.add_css_class("job-action");
        cancel.set_tooltip_text(Some("Cancel download"));
        cancel.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            14,
        )));
        cancel.connect_clicked(move |_| on_cancel());
        chip.append(&cancel);

        self.overlay.add_overlay(&chip);
        self.download_chip.replace(Some(DownloadChip {
            root: chip,
            overlay: self.overlay.clone(),
            status,
            progress,
            indeterminate: Rc::new(Cell::new(true)),
            pulse_source: Rc::new(RefCell::new(None)),
        }));
        if let Some(chip) = self.download_chip.borrow().as_ref() {
            ensure_indeterminate_pulse(&chip.progress, &chip.indeterminate, &chip.pulse_source);
        }
    }

    pub(super) fn update_download_progress(&self, downloaded: u64, total_bytes: Option<u64>) {
        let chip_ref = self.download_chip.borrow();
        let Some(chip) = chip_ref.as_ref() else {
            return;
        };
        match total_bytes.filter(|total| *total > 0) {
            Some(total) => {
                let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                let percentage = (fraction * 100.0) as usize;
                chip.status
                    .set_text(&format!("{percentage}% of {}", format_file_size(total)));
                chip.indeterminate.set(false);
                chip.progress.set_fraction(fraction);
            }
            None => {
                chip.status
                    .set_text(&format!("{} downloaded", format_file_size(downloaded)));
                chip.indeterminate.set(true);
            }
        }
    }

    pub(super) fn dismiss_download_progress(&self) {
        if let Some(chip) = self.download_chip.take() {
            chip.indeterminate.set(false);
            if let Some(source) = chip.pulse_source.take() {
                source.remove();
            }
            chip.overlay.remove_overlay(&chip.root);
        }
    }
}

pub(super) struct DownloadChip {
    root: gtk::Box,
    overlay: gtk::Overlay,
    status: gtk::Label,
    progress: gtk::ProgressBar,
    indeterminate: Rc<Cell<bool>>,
    pulse_source: Rc<RefCell<Option<glib::SourceId>>>,
}

fn ensure_indeterminate_pulse(
    progress: &gtk::ProgressBar,
    indeterminate: &Rc<Cell<bool>>,
    pulse_source: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    if pulse_source.borrow().is_some() {
        return;
    }
    let weak_progress = progress.downgrade();
    let indeterminate = indeterminate.clone();
    let pulse_source = pulse_source.clone();
    let holder = pulse_source.clone();
    let source = glib::timeout_add_local(INDETERMINATE_PROGRESS_INTERVAL, move || {
        if !indeterminate.get() {
            holder.borrow_mut().take();
            return glib::ControlFlow::Break;
        }
        let Some(progress) = weak_progress.upgrade() else {
            holder.borrow_mut().take();
            return glib::ControlFlow::Break;
        };
        progress.pulse();
        glib::ControlFlow::Continue
    });
    pulse_source.replace(Some(source));
}
