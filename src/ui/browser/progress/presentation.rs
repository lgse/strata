// SPDX-License-Identifier: MIT

use super::{FileProgressState, FileProgressView};
use crate::ui::modal::close_on_escape;
use crate::ui::progress_dock::CompactProgress;
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn install_progress_keys(layer: &gtk::Box, action: &Rc<dyn Fn()>) {
    let action = action.clone();
    close_on_escape(layer, move || action());
}

impl FileProgressState {
    pub(super) fn attach_dock(self: &Rc<Self>, view: &mut FileProgressView) {
        let compact = Rc::new(CompactProgress::new(&view.overlay, &view.icon_name));
        let weak = Rc::downgrade(self);
        compact.set_cancel_action(Rc::new(move || {
            if let Some(state) = weak.upgrade()
                && !state.transfer_cancel_requested.get()
            {
                let action = state
                    .file_progress_view
                    .borrow()
                    .as_ref()
                    .and_then(|view| view.cancel_action.clone());
                if let Some(action) = action {
                    action();
                }
            }
        }));
        view.compact = Some(compact);
        self.sync_compact(view);
    }

    pub(super) fn sync_compact(&self, view: &FileProgressView) {
        let Some(compact) = &view.compact else {
            return;
        };
        compact.title.set_text(&view.title.text());
        let transferring = view.transfer_header.is_visible();
        let status = if view.indeterminate.get() {
            "…".to_owned()
        } else if transferring {
            view.transfer_percent.text().to_string()
        } else {
            crate::i18n::percent((view.progress.fraction() * 100.0) as usize)
        };
        compact.status.set_text(&status);
        if self.transfer_cancel_requested.get() && transferring {
            compact.info.set_text(&crate::i18n::tr(
                "Device may still be writing. Do not unplug until the operation stops.",
            ));
        } else if transferring {
            let file = self.transfer_current_file.borrow();
            let description = self.task_description.borrow();
            compact.info.set_text(
                file.as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&description),
            );
        } else {
            compact.info.set_text(&self.task_description.borrow());
        }
        let counted = self
            .transfer_progress
            .get()
            .filter(|_| transferring)
            .map(|snapshot| {
                super::transfer_count(
                    snapshot.completed_items,
                    self.file_operation_progress.get().1,
                    snapshot.completed_files,
                    snapshot.total_files,
                )
            });
        let preparing = matches!(counted, Some(None));
        let count = counted.flatten().map(|(completed, total, _)| {
            format!(
                "{}/{}",
                crate::i18n::integer(completed as u64),
                crate::i18n::integer(total as u64)
            )
        });
        compact.count.set_text(count.as_deref().unwrap_or_default());
        compact.count.set_visible(count.is_some());
        let destination = self.destination_description.borrow();
        let label = self.destination_label.borrow();
        compact
            .destination
            .set_text(if destination.is_empty() { "" } else { &label });
        crate::ui::accessibility::set_description(&compact.destination, Some(&destination));
        compact.destination.set_visible(!destination.is_empty());
        let meta = if preparing {
            crate::i18n::tr("Preparing items…")
        } else if transferring {
            view.transfer_rate.text().into()
        } else {
            view.status.text().into()
        };
        let show_meta = transferring
            || meta != crate::i18n::percent((view.progress.fraction() * 100.0) as usize);
        compact.meta.set_text(if show_meta { &meta } else { "" });
        if !view.indeterminate.get() {
            compact.progress.set_fraction(view.progress.fraction());
        }
        compact
            .cancel
            .set_sensitive(!self.transfer_cancel_requested.get());
        compact
            .cancel
            .set_tooltip_text(Some(&crate::i18n::tr("Cancel operation")));
        crate::ui::accessibility::set_label(&compact.cancel, &view.cancel_label);
    }
}

/// `title` is the untranslated progress title passed by the operation.
pub(super) fn cancel_accessible_label(title: &str) -> String {
    crate::i18n::tr(match title {
        "Copying items" => "Cancel copying",
        "Moving items" => "Cancel moving",
        "Deleting items" => "Cancel deleting",
        "Restoring items" => "Cancel restoring",
        "Compressing items" => "Cancel compressing",
        "Processing archive…" => "Cancel archive processing",
        "Emptying Trash" => "Cancel emptying Trash",
        _ => "Cancel operation",
    })
}
