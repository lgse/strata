// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc, time::Duration};

use gtk::{glib, prelude::*};

const GRACE_PERIOD: Duration = Duration::from_millis(150);

struct PendingLoad(RefCell<Option<glib::SourceId>>);

impl PendingLoad {
    fn cancel(&self) {
        if let Some(source) = self.0.borrow_mut().take() {
            source.remove();
        }
    }
}

impl Drop for PendingLoad {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Clone)]
pub(crate) struct DelayedLoading {
    stack: gtk::Stack,
    pending: Rc<PendingLoad>,
}

impl DelayedLoading {
    pub(crate) fn new(stack: &gtk::Stack) -> Self {
        stack.add_named(
            &gtk::Box::new(gtk::Orientation::Vertical, 0),
            Some("pending"),
        );
        let loading = Self {
            stack: stack.clone(),
            pending: Rc::new(PendingLoad(RefCell::new(None))),
        };
        loading.start();
        loading
    }

    pub(crate) fn start(&self) {
        self.pending.cancel();
        // A reload must not crossfade stale rows or a previous skeleton into the grace period.
        let transition = self.stack.transition_type();
        self.stack
            .set_transition_type(gtk::StackTransitionType::None);
        self.stack.set_visible_child_name("pending");
        self.stack.set_transition_type(transition);
        let stack = self.stack.downgrade();
        let pending = Rc::downgrade(&self.pending);
        *self.pending.0.borrow_mut() =
            Some(glib::timeout_add_local_once(GRACE_PERIOD, move || {
                if let Some(pending) = pending.upgrade() {
                    pending.0.borrow_mut().take();
                    if let Some(stack) = stack.upgrade() {
                        stack.set_visible_child_name("loading");
                    }
                }
            }));
    }

    pub(crate) fn show(&self, name: &str) {
        self.pending.cancel();
        self.stack.set_visible_child_name(name);
    }
}

#[cfg(test)]
mod tests;
