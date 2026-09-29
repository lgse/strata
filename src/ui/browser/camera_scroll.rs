// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

pub(crate) struct CameraTopAnchor(gtk::Widget);

impl CameraTopAnchor {
    pub(crate) fn capture(view: &gtk::Widget) -> Option<Self> {
        if !view.is_mapped() || view.height() <= 1 {
            return None;
        }
        let scroll = view
            .ancestor(gtk::ScrolledWindow::static_type())?
            .downcast::<gtk::ScrolledWindow>()
            .ok()?;
        let adjustment = scroll.vadjustment();
        (adjustment.value() <= adjustment.lower() + 0.5).then(|| Self(view.clone()))
    }

    pub(crate) fn restore(self) {
        let Some(scroll) = self
            .0
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
        else {
            return;
        };
        let cancelled = Rc::new(Cell::new(false));
        let input = gtk::EventControllerLegacy::new();
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        let cancel = cancelled.clone();
        input.connect_event(move |_, event| {
            if matches!(
                event.event_type(),
                gtk::gdk::EventType::KeyPress
                    | gtk::gdk::EventType::ButtonPress
                    | gtk::gdk::EventType::Scroll
                    | gtk::gdk::EventType::TouchBegin
            ) {
                cancel.set(true);
            }
            glib::Propagation::Proceed
        });
        scroll.add_controller(input.clone());
        let cancel = cancelled.clone();
        let unmapped = RefCell::new(Some(scroll.connect_unmap(move |_| cancel.set(true))));
        let adjustment = scroll.vadjustment();
        let frames = Cell::new(0);
        // Section headers are allocated after the model notification. Restore
        // through that layout, but let any user input cancel the pending work.
        scroll.add_tick_callback(move |scroll, _| {
            if cancelled.get() || frames.get() == 2 {
                if let Some(unmapped) = unmapped.take() {
                    scroll.disconnect(unmapped);
                }
                scroll.remove_controller(&input);
                return glib::ControlFlow::Break;
            }
            frames.set(frames.get() + 1);
            adjustment.set_value(adjustment.lower());
            glib::ControlFlow::Continue
        });
        // GTK anchors the old first item when sorted camera batches prepend rows.
        // Anchor the first position instead, without moving focus or selection.
        let info = gtk::ScrollInfo::new();
        info.set_enable_horizontal(false);
        if let Some(list) = self.0.downcast_ref::<gtk::ListView>() {
            if list.model().is_some_and(|model| model.n_items() > 0) {
                list.scroll_to(0, gtk::ListScrollFlags::NONE, Some(info));
            }
        } else if let Some(grid) = self.0.downcast_ref::<gtk::GridView>()
            && grid.model().is_some_and(|model| model.n_items() > 0)
        {
            grid.scroll_to(0, gtk::ListScrollFlags::NONE, Some(info));
        }
    }
}
