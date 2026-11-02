// SPDX-License-Identifier: MIT

use crate::ui::browser::entry::item_count_label;
use crate::ui::modal::ModalHost;
use gtk::glib;
use gtk::prelude::*;
#[cfg(test)]
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

mod context;
mod dismiss;
mod presentation;
mod status;
mod transfer_updates;
mod view;
pub(super) use context::{BackgroundProgress, FileProgressState, file_operation_cancel};
pub(in crate::ui::browser) use status::cancelled_operation_summary;
pub(super) use status::{TRANSFER_SUBTITLE, transfer_heading};
use status::{transfer_count, transfer_progress_status, transfer_rate_status};
pub(super) use view::FileProgressView;
use view::ProgressHeading;

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

#[cfg(test)]
pub(super) fn reset_file_progress_delay_for_test() {
    FILE_PROGRESS_DELAY_OVERRIDE.with(|cell| cell.set(None));
}

const INDETERMINATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const STALLED_CANCEL_DELAY: Duration = Duration::from_secs(8);

const PROGRESS_THROTTLE_INTERVAL: Duration = Duration::from_millis(33);

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

impl FileProgressState {
    pub(super) fn show_file_operation_progress(
        self: &Rc<Self>,
        total: usize,
        icon: &str,
        title_text: &str,
        subtitle_text: &str,
        on_cancel: Rc<dyn Fn()>,
    ) {
        let at_once = should_show_progress_immediately(total);
        self.show_operation_progress(total, at_once, icon, title_text, subtitle_text, on_cancel);
    }

    pub(super) fn show_operation_progress(
        self: &Rc<Self>,
        total: usize,
        at_once: bool,
        icon: &str,
        title_text: &str,
        subtitle_text: &str,
        on_cancel: Rc<dyn Fn()>,
    ) {
        self.dismiss_file_operation_progress();
        self.file_operation_progress.set((0, total));
        let heading = ProgressHeading::new(icon, title_text, subtitle_text);
        if at_once {
            self.present_file_operation_progress(&heading, on_cancel);
            return;
        }
        self.present_file_operation_progress_later(heading, on_cancel);
    }

    pub(super) fn is_presented(&self) -> bool {
        self.file_progress_view.borrow().is_some()
    }

    /// Appears only if the conflict check outlasts the usual progress delay, where the transfer's
    /// own row will take over.
    pub(super) fn show_transfer_preparation(
        self: &Rc<Self>,
        moving: bool,
        (description, (destination, label)): (String, (String, String)),
        on_cancel: Rc<dyn Fn()>,
    ) {
        self.dismiss_file_operation_progress();
        self.dock_only.set(!moving);
        self.task_description.replace(description);
        self.destination_description.replace(destination);
        self.destination_label.replace(label);
        self.transfer_progress.set(Some(TransferProgressSnapshot {
            completed_items: 0,
            completed_files: 0,
            total_files: None,
            transferred_bytes: 0,
            total_bytes: None,
        }));
        let (icon, title) = transfer_heading(moving);
        let heading = ProgressHeading::new(icon, title, TRANSFER_SUBTITLE);
        self.present_file_operation_progress_later(heading, on_cancel);
    }

    fn present_file_operation_progress_later(
        self: &Rc<Self>,
        heading: ProgressHeading,
        on_cancel: Rc<dyn Fn()>,
    ) {
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(file_progress_delay(), move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.pending_file_progress.borrow_mut().take();
            state.present_file_operation_progress(&heading, on_cancel);
        });
        self.pending_file_progress.replace(Some(source));
    }

    fn present_file_operation_progress(
        self: &Rc<Self>,
        heading: &ProgressHeading,
        on_cancel: Rc<dyn Fn()>,
    ) {
        // Blurring the live listing repaints the whole window on each operation update.
        let Some(host) = ModalHost::for_widget(&self.overlay) else {
            return;
        };
        let view = FileProgressView::build(host, heading, self.dock_only.get());
        let cancel = view.cancel.clone();
        self.file_progress_view.replace(Some(view));

        let cancel_action = self.operation_cancel_action(on_cancel);
        let click_action = cancel_action.clone();
        cancel.connect_clicked(move |_| click_action());
        self.install_cancel_action(&cancel_action);
        cancel.grab_focus();
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            view.ensure_indeterminate_pulse();
        }
        self.render_current_progress();
        if let Some(on_present) = self.on_present.take() {
            on_present();
        }
    }

    fn operation_cancel_action(self: &Rc<Self>, on_cancel: Rc<dyn Fn()>) -> Rc<dyn Fn()> {
        let weak = Rc::downgrade(self);
        Rc::new(move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.transfer_progress.get().is_some() {
                state.request_transfer_cancel(&on_cancel);
            } else if !state.transfer_cancel_requested.replace(true) {
                state.show_operation_cancelling();
                on_cancel();
            }
        })
    }

    fn show_operation_cancelling(&self) {
        if let Some(view) = self.file_progress_view.borrow().as_ref() {
            view.cancel.set_sensitive(false);
            view.title
                .set_text(&crate::i18n::tr("Cancelling operation…"));
            view.indeterminate.set(true);
            view.ensure_indeterminate_pulse();
            self.sync_compact(view);
        }
    }

    fn install_cancel_action(self: &Rc<Self>, cancel_action: &Rc<dyn Fn()>) {
        if let Some(view) = self.file_progress_view.borrow_mut().as_mut() {
            view.cancel_action = Some(cancel_action.clone());
            if self.dock_only.get() {
                self.attach_dock(view);
            }
            if let Some(layer) = view.layer.borrow().as_ref() {
                presentation::install_progress_keys(layer, cancel_action);
            }
        }
    }

    fn render_current_progress(self: &Rc<Self>) {
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
        let new_status = crate::i18n::percent(pct);
        let new_fraction = completed as f64 / total.max(1) as f64;
        if view.status.text() == new_status && view.progress.fraction() == new_fraction {
            return;
        }
        view.status.set_text(&new_status);
        view.indeterminate.set(false);
        view.progress.set_fraction(new_fraction);
        self.sync_compact(view);
    }

    /// A single item stays at 0% until it is gone, so pulse and show the entries removed
    /// instead. With several items the item count already carries the progress.
    pub(super) fn update_removed_entries(&self, removed: usize) {
        let (_, total) = self.file_operation_progress.get();
        if total > 1 || self.transfer_cancel_requested.get() {
            return;
        }
        let progress_view = self.file_progress_view.borrow();
        let Some(view) = progress_view.as_ref() else {
            return;
        };
        view.status.set_text(&rust_i18n::t!(
            "%{items} deleted",
            items = item_count_label(removed)
        ));
        view.indeterminate.set(true);
        view.ensure_indeterminate_pulse();
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
            view.status.set_text(&crate::i18n::tr("Stopping…"));
            view.indeterminate.set(true);
        } else if completed == 0 {
            view.status
                .set_text(&crate::i18n::tr(self.archive_preparing_status(total)));
            view.indeterminate.set(true);
        } else if total == 0 {
            view.status
                .set_text(&crate::i18n::count("files", completed));
            view.indeterminate.set(true);
        } else {
            view.status.set_text(&rust_i18n::t!(
                "%{completed} / %{items}",
                completed = crate::i18n::integer(completed as u64),
                items = crate::i18n::count("files", total)
            ));
            view.indeterminate.set(false);
            view.progress.set_fraction(completed as f64 / total as f64);
        }
        view.ensure_indeterminate_pulse();
        self.sync_compact(view);
    }

    fn archive_preparing_status(&self, total: usize) -> &'static str {
        if total == 0 {
            "Preparing…"
        } else if self.archive_compressing.get() {
            "Compressing…"
        } else {
            "Processing archive…"
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
        view.status.set_text(&rust_i18n::t!(
            "%{items} deleted",
            items = item_count_label(processed)
        ));
        view.indeterminate.set(true);
        self.sync_compact(view);
    }
}
