// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

/// Dispatches outside GTK layout, even when scheduled by a frame callback.
pub(super) struct FrameTask {
    cancelled: Rc<Cell<bool>>,
}

impl FrameTask {
    pub(super) fn new(widget: Option<&gtk::Widget>, work: impl FnOnce() + 'static) -> Self {
        let cancelled = Rc::new(Cell::new(false));
        let flag = cancelled.clone();
        let dispatch = move || {
            glib::idle_add_local_once(move || {
                if !flag.replace(true) {
                    work();
                }
            });
        };
        if let Some(widget) = widget.filter(|widget| widget.is_mapped()) {
            let dispatch = RefCell::new(Some(dispatch));
            widget.add_tick_callback(move |_, _| {
                if let Some(dispatch) = dispatch.borrow_mut().take() {
                    dispatch();
                }
                glib::ControlFlow::Break
            });
        } else {
            // Unmapped/headless producers have no advancing frame clock.
            dispatch();
        }
        Self { cancelled }
    }
}

impl Drop for FrameTask {
    fn drop(&mut self) {
        self.cancelled.set(true);
    }
}

#[cfg(test)]
mod tests;
