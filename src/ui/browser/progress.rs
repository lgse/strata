// SPDX-License-Identifier: MIT

use crate::ui::browser::ViewState;
use crate::ui::browser::entry::{format_file_size, item_count_label};
use crate::ui::modal::{animate_in, dismiss_modal_layer_then};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const FILE_PROGRESS_DELAY: Duration = Duration::from_millis(350);

const FILE_PROGRESS_DONE_LINGER: Duration = Duration::from_millis(700);

const INDETERMINATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

const IMMEDIATE_PROGRESS_ITEM_COUNT: usize = 16;

fn should_show_progress_immediately(total: usize) -> bool {
    total == 0 || total >= IMMEDIATE_PROGRESS_ITEM_COUNT
}

pub(super) struct FileProgressView {
    layer: gtk::Box,
    overlay: gtk::Overlay,
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
        on_cancel: Rc<dyn Fn()>,
    ) {
        self.dismiss_file_operation_progress();
        self.file_operation_progress.set((0, total));
        if should_show_progress_immediately(total) {
            self.present_file_operation_progress(icon, title_text, on_cancel);
            return;
        }

        let weak = Rc::downgrade(self);
        let icon = icon.to_owned();
        let title_text = title_text.to_owned();
        let source = glib::timeout_add_local_once(FILE_PROGRESS_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.pending_file_progress.borrow_mut().take();
            state.present_file_operation_progress(&icon, &title_text, on_cancel);
        });
        self.pending_file_progress.replace(Some(source));
    }

    fn present_file_operation_progress(
        self: &Rc<Self>,
        icon: &str,
        title_text: &str,
        on_cancel: Rc<dyn Fn()>,
    ) {
        let overlay = self.overlay.clone();

        let bezel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bezel.add_css_class("job-icon-bezel");
        bezel.set_valign(gtk::Align::Center);
        bezel.append(&crate::assets::primary_icon(icon, 16));
        let title = gtk::Label::new(Some(title_text));
        title.add_css_class("job-name");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let archive_activity = gtk::Spinner::new();
        archive_activity.set_visible(false);
        let status = gtk::Label::new(Some("0%"));
        status.add_css_class("job-status");
        let cancel = gtk::Button::new();
        cancel.add_css_class("job-action");
        cancel.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            14,
        )));
        cancel.set_tooltip_text(Some("Cancel"));
        cancel.update_property(&[gtk::accessible::Property::Label("Cancel")]);
        cancel.set_valign(gtk::Align::Center);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&title);
        head.append(&archive_activity);
        head.append(&status);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("job-progress");
        progress.set_fraction(0.0);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail.set_hexpand(true);
        detail.set_valign(gtk::Align::Center);
        detail.append(&head);
        detail.append(&progress);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.add_css_class("file-progress-toast");
        content.append(&bezel);
        content.append(&detail);
        content.append(&cancel);

        let indeterminate = Rc::new(Cell::new(false));
        let pulse_source = Rc::new(RefCell::new(None));

        let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layer.add_css_class("file-progress-card");
        layer.set_halign(gtk::Align::End);
        layer.set_valign(gtk::Align::End);
        layer.append(&content);
        overlay.add_overlay(&layer);
        animate_in(&layer);
        self.file_progress_view.replace(Some(FileProgressView {
            layer,
            overlay,
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
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            ensure_indeterminate_pulse(view);
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
        ensure_indeterminate_pulse(view);
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

    /// Terminal events land the same tick the last progress update does -- without
    /// a beat at 100% the card vanishes before the finished state is ever visible.
    pub(super) fn complete_file_operation_progress_then(
        self: &Rc<Self>,
        after_dismiss: impl FnOnce() + 'static,
    ) {
        if self.file_progress_view.borrow().is_none() {
            self.dismiss_file_operation_progress_then(after_dismiss);
            return;
        }
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            view.indeterminate.set(false);
            view.archive_activity.stop();
            view.progress.set_fraction(1.0);
            view.status.set_text("Done");
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(FILE_PROGRESS_DONE_LINGER, move || {
            if let Some(state) = weak.upgrade() {
                state.dismiss_file_operation_progress_then(after_dismiss);
            }
        });
    }

    pub(super) fn complete_file_operation_progress(self: &Rc<Self>) {
        self.complete_file_operation_progress_then(|| {});
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
            dismiss_modal_layer_then(&view.layer, &view.overlay, None, after_dismiss);
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
}

fn ensure_indeterminate_pulse(view: &FileProgressView) {
    if view.pulse_source.borrow().is_some() {
        return;
    }
    let weak_progress = view.progress.downgrade();
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
        glib::ControlFlow::Continue
    });
    view.pulse_source.replace(Some(source));
}

#[cfg(test)]
mod tests;
