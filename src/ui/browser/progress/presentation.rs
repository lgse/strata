// SPDX-License-Identifier: MIT

use super::{FileProgressState, FileProgressView};
use crate::ui::progress_dock::CompactProgress;
use gtk::{glib, prelude::*};
use std::rc::Rc;

pub(super) fn install_progress_keys(layer: &gtk::Box, action: &Rc<dyn Fn()>) {
    let action = action.clone();
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            action();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
}

impl FileProgressState {
    pub(super) fn attach_dock(self: &Rc<Self>, view: &mut FileProgressView) {
        let compact = Rc::new(CompactProgress::new(&view.overlay, &view.icon_name));
        let weak = Rc::downgrade(self);
        compact.cancel.connect_clicked(move |_| {
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
        });
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
            format!("{}%", (view.progress.fraction() * 100.0) as usize)
        };
        compact.status.set_text(&status);
        if self.transfer_cancel_requested.get() && transferring {
            compact
                .info
                .set_text("Device may still be writing. Do not unplug until the operation stops.");
        } else if transferring {
            compact.info.set_text(&view.transfer_items.text());
        } else {
            compact.info.set_text(&self.task_description.borrow());
        }
        compact
            .destination
            .set_text(&self.destination_description.borrow());
        compact
            .destination
            .set_visible(!self.destination_description.borrow().is_empty());
        let meta = if transferring {
            view.transfer_rate.text()
        } else {
            view.status.text()
        };
        compact.meta.set_text(&meta);
        if !view.indeterminate.get() {
            compact.progress.set_fraction(view.progress.fraction());
        }
        compact
            .cancel
            .set_sensitive(!self.transfer_cancel_requested.get());
        compact.cancel.set_tooltip_text(Some("Cancel operation"));
        crate::ui::accessibility::set_label(
            &compact.cancel,
            &format!("Cancel {}", view.title.text()),
        );
    }
}
