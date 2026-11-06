// SPDX-License-Identifier: MIT

//! A first visit to a folder, until the user's own input reaches its pane.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

/// Watches a pane opened with no saved position. Its load may still move focus to the
/// first row shown, but only while no pointer, scroll, touch or key input reached the
/// pane: a user who scrolled or moved keeps the view where they left it.
pub(super) struct FirstVisit {
    untouched: Rc<Cell<bool>>,
    shell: glib::WeakRef<gtk::Widget>,
    input: gtk::EventControllerLegacy,
}

impl FirstVisit {
    pub(super) fn watch(shell: &impl IsA<gtk::Widget>) -> Self {
        let untouched = Rc::new(Cell::new(true));
        let input = gtk::EventControllerLegacy::new();
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        let flag = untouched.clone();
        input.connect_event(move |_, event| {
            if matches!(
                event.event_type(),
                gtk::gdk::EventType::ButtonPress
                    | gtk::gdk::EventType::Scroll
                    | gtk::gdk::EventType::TouchBegin
                    | gtk::gdk::EventType::KeyPress
            ) {
                flag.set(false);
            }
            glib::Propagation::Proceed
        });
        shell.as_ref().add_controller(input.clone());
        Self {
            untouched,
            shell: shell.as_ref().downgrade(),
            input,
        }
    }

    pub(super) fn untouched(&self) -> bool {
        self.untouched.get()
    }
}

impl Drop for FirstVisit {
    fn drop(&mut self) {
        if let Some(shell) = self.shell.upgrade() {
            shell.remove_controller(&self.input);
        }
    }
}
