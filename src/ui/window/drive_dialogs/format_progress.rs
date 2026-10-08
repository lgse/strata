// SPDX-License-Identifier: MIT

use super::drive_ops;
use crate::{
    assets,
    ui::{modal::ModalHost, progress_dock::CompactProgress},
};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub(super) struct FormatProgress {
    card: Rc<CompactProgress>,
    pulse: RefCell<Option<glib::SourceId>>,
    finished: Cell<bool>,
    display_name: String,
    overlay: gtk::Overlay,
}

impl FormatProgress {
    pub(super) fn new(parent: &gtk::Widget, display_name: &str) -> Option<Rc<Self>> {
        let host = ModalHost::for_widget(parent)?;
        let card = Rc::new(CompactProgress::new(&host.overlay, assets::icons::SHREDDER));
        card.title.set_text(&crate::i18n::tr("Formatting drive"));
        card.status.set_text("…");
        card.destination.set_text(&rust_i18n::t!(
            "Drive: %{display_name}",
            display_name = display_name
        ));
        card.info
            .set_text(&crate::i18n::tr("Do not unplug until formatting finishes."));
        card.meta.set_text(&crate::i18n::tr("Formatting…"));
        card.cancel.set_visible(false);
        let state = Rc::new(Self {
            card,
            pulse: RefCell::new(None),
            finished: Cell::new(false),
            display_name: display_name.to_owned(),
            overlay: host.overlay,
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
            crate::ui::close_guard::install(&window, move |_| {
                let state = weak.upgrade()?;
                (!state.finished.get()).then(|| crate::ui::close_guard::CloseBlocker {
                    title: crate::i18n::tr("Drive formatting is still active"),
                    detail: crate::i18n::tr("Wait until formatting finishes before closing this window. Do not unplug the drive."),
                })
            });
            let weak = Rc::downgrade(&state);
            window.connect_unrealize(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.dismiss();
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
                self.card
                    .info
                    .set_text(&crate::i18n::tr("The drive was formatted successfully."));
                self.card.completed("Format complete");
            }
            Err(error) => {
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
