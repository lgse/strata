// SPDX-License-Identifier: MIT

//! A first visit to a folder, until the user's own input reaches its pane.

use std::{cell::Cell, rc::Rc};

use gtk::{gdk, glib, prelude::*};

pub(super) fn is_user_input(event: &gdk::Event) -> bool {
    matches!(
        event.event_type(),
        gdk::EventType::ButtonPress
            | gdk::EventType::Scroll
            | gdk::EventType::TouchBegin
            | gdk::EventType::KeyPress
    )
}

/// Watches the pane a navigation opened with no saved position. Its load may still
/// move focus to the first row shown, but only while no user input reached the pane:
/// a user who scrolled or moved keeps the view where they left it.
pub(super) struct FirstVisit {
    depth: usize,
    untouched: Untouched,
    input: Option<(glib::WeakRef<gtk::Widget>, gtk::EventControllerLegacy)>,
}

/// Whether no user input reached the pane yet; the input controller holds a copy.
#[derive(Clone)]
struct Untouched(Rc<Cell<bool>>);

impl Untouched {
    fn interrupt(&self) {
        self.0.set(false);
    }

    fn get(&self) -> bool {
        self.0.get()
    }
}

impl FirstVisit {
    pub(super) fn new(depth: usize) -> Self {
        Self {
            depth,
            untouched: Untouched(Rc::new(Cell::new(true))),
            input: None,
        }
    }

    pub(super) fn watch(shell: &impl IsA<gtk::Widget>, depth: usize) -> Self {
        let mut visit = Self::new(depth);
        let untouched = visit.untouched.clone();
        let input = gtk::EventControllerLegacy::new();
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        input.connect_event(move |_, event| {
            if is_user_input(event) {
                untouched.interrupt();
            }
            glib::Propagation::Proceed
        });
        shell.as_ref().add_controller(input.clone());
        visit.input = Some((shell.as_ref().downgrade(), input));
        visit
    }

    /// Ends the visit in `slot` when its load at `depth` finished, reporting whether no
    /// input reached its pane. Another column's load leaves the visit waiting.
    pub(super) fn take_untouched(slot: &mut Option<Self>, depth: usize) -> bool {
        if slot.as_ref().is_none_or(|visit| visit.depth != depth) {
            return false;
        }
        slot.take().is_some_and(|visit| visit.untouched.get())
    }
}

impl Drop for FirstVisit {
    fn drop(&mut self) {
        if let Some((shell, input)) = self.input.take()
            && let Some(shell) = shell.upgrade()
        {
            shell.remove_controller(&input);
        }
    }
}

#[cfg(test)]
mod tests;
