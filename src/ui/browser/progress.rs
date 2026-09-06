// SPDX-License-Identifier: GPL-3.0-or-later

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
        layout.body.append(&status);
        layout.body.append(&progress);
        let content = layout.content;
        let cancel = layout.confirm;

        let indeterminate = Rc::new(Cell::new(false));
        let pulse_source = Rc::new(RefCell::new(None));
        let weak_progress = progress.downgrade();
        let indeterminate_for_pulse = indeterminate.clone();
        let source_for_pulse = pulse_source.clone();
        let source = glib::timeout_add_local(INDETERMINATE_PROGRESS_INTERVAL, move || {
            if !indeterminate_for_pulse.get() {
                source_for_pulse.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            let Some(progress) = weak_progress.upgrade() else {
                source_for_pulse.borrow_mut().take();
                return glib::ControlFlow::Break;
            };
            progress.pulse();
            glib::ControlFlow::Continue
        });
        pulse_source.replace(Some(source));

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        self.file_progress_view.replace(Some(FileProgressView {
            layer,
            overlay: window_overlay,
            blurred_root,
            progress,
            status,
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
        if let Some((completed_items, transferred_bytes, total_bytes)) =
            self.transfer_progress.get()
        {
            self.update_transfer_progress(completed_items, transferred_bytes, total_bytes);
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
        if total == 0 {
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
        if let Some(source) = self.pending_file_progress.take() {
            source.remove();
        }
        self.file_operation_progress.set((0, 0));
        self.transfer_progress.set(None);
        if let Some(view) = self.file_progress_view.take() {
            view.indeterminate.set(false);
            if let Some(source) = view.pulse_source.take() {
                source.remove();
            }
            dismiss_modal_layer(&view.layer, &view.overlay, view.blurred_root.as_ref());
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
}

#[cfg(test)]
mod tests;
