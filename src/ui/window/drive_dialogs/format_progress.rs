// SPDX-License-Identifier: MIT

use super::drive_ops;
use crate::{
    assets,
    ui::{modal::ModalHost, progress_dock::CompactProgress},
};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

pub(super) struct FormatProgress {
    card: Rc<CompactProgress>,
    pulse: RefCell<Option<glib::SourceId>>,
    finished: Cell<bool>,
    display_name: String,
    owner: Weak<RefCell<Option<Rc<Self>>>>,
    overlay: gtk::Overlay,
}

impl FormatProgress {
    pub(super) fn new(parent: &gtk::Widget, display_name: &str) -> Option<Rc<Self>> {
        let host = ModalHost::for_widget(parent)?;
        let card = Rc::new(CompactProgress::new(&host.overlay, assets::icons::SHREDDER));
        card.title.set_text("Formatting drive");
        card.status.set_text("…");
        card.destination.set_text(&format!("Drive: {display_name}"));
        card.info
            .set_text("Do not unplug until formatting finishes.");
        card.meta.set_text("Formatting…");
        card.cancel.set_visible(false);
        let owner = Rc::new(RefCell::new(None));
        let state = Rc::new(Self {
            card,
            pulse: RefCell::new(None),
            finished: Cell::new(false),
            display_name: display_name.to_owned(),
            owner: Rc::downgrade(&owner),
            overlay: host.overlay,
        });
        owner.replace(Some(state.clone()));
        let retained = owner.clone();
        state.card.cancel.connect_clicked(move |_| {
            let state = retained.borrow().clone();
            if let Some(state) = state
                && state.finished.get()
            {
                retained.borrow_mut().take();
                state.dismiss();
            }
        });
        let weak = Rc::downgrade(&state);
        state.pulse.replace(Some(glib::timeout_add_local(
            Duration::from_millis(80),
            move || {
                let Some(state) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                state.card.progress.pulse();
                glib::ControlFlow::Continue
            },
        )));
        if let Some(window) = state.overlay.root().and_downcast::<gtk::Window>() {
            let weak = Rc::downgrade(&state);
            window.connect_close_request(move |window| {
                if let Some(state) = weak.upgrade() && !state.finished.get() {
                    crate::ui::window::show_error_dialog(window, "Drive formatting is still active", "Wait until formatting finishes before closing this window. Do not unplug the drive.");
                    glib::Propagation::Stop
                } else { glib::Propagation::Proceed }
            });
            let weak_owner = Rc::downgrade(&owner);
            window.connect_unrealize(move |_| {
                if let Some(owner) = weak_owner.upgrade() {
                    let state = owner.borrow_mut().take();
                    if let Some(state) = state {
                        state.dismiss();
                    }
                }
            });
        }
        Some(state)
    }

    fn dismiss(&self) {
        if let Some(source) = self.pulse.borrow_mut().take() {
            source.remove();
        }
        self.card.remove();
    }

    pub(super) fn complete(&self, result: Result<(), drive_ops::DriveOpError>) {
        self.finished.set(true);
        if let Some(source) = self.pulse.borrow_mut().take() {
            source.remove();
        }
        match result {
            Ok(()) => {
                self.card.title.set_text("Format complete");
                self.card.status.set_text("Done");
                self.card
                    .info
                    .set_text("The drive was formatted successfully.");
                self.card
                    .meta
                    .set_text("Click the drive in the sidebar to mount it.");
                self.card.progress.set_fraction(1.0);
                self.card.cancel.set_visible(true);
                self.card
                    .cancel
                    .set_tooltip_text(Some("Close formatting result"));
                crate::ui::accessibility::set_label(&self.card.cancel, "Close formatting result");
            }
            Err(error) => {
                if let Some(owner) = self.owner.upgrade() {
                    owner.borrow_mut().take();
                }
                self.dismiss();
                if self.overlay.root().is_some() {
                    drive_ops::report_result(
                        &self.overlay.clone().upcast::<gtk::Widget>(),
                        &self.display_name,
                        Err(error),
                    );
                }
            }
        }
    }
}
