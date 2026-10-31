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

pub(super) fn after_layout<W: IsA<gtk::Widget>>(widget: &W, work: impl Fn(&W) + 'static) {
    let work = Rc::new(work);
    let connection = Rc::new(RefCell::new(
        None::<(gtk::gdk::FrameClock, glib::SignalHandlerId)>,
    ));
    let connect = {
        let connection = connection.clone();
        move |widget: &W| {
            if let Some((clock, handler)) = connection.borrow_mut().take() {
                clock.disconnect(handler);
            }
            let Some(clock) = widget.frame_clock() else {
                return;
            };
            let weak = widget.downgrade();
            let work = work.clone();
            let handler = clock.connect_layout(move |_| {
                if let Some(widget) = weak.upgrade() {
                    work(&widget);
                }
            });
            connection.replace(Some((clock, handler)));
        }
    };
    if widget.is_realized() {
        connect(widget);
    }
    widget.connect_realize(connect);
    widget.connect_unrealize(move |_| {
        if let Some((clock, handler)) = connection.borrow_mut().take() {
            clock.disconnect(handler);
        }
    });
}

pub(super) fn tick_while_active<W: IsA<gtk::Widget>>(
    widget: &W,
    idle_frames: u32,
    work: impl Fn(&W) + 'static,
) {
    let remaining = Rc::new(Cell::new(0_u32));
    let running = Rc::new(Cell::new(false));
    let work = Rc::new(work);
    let kick = {
        let remaining = remaining.clone();
        let running = running.clone();
        let work = work.clone();
        move |widget: &W| {
            remaining.set(idle_frames);
            if running.replace(true) {
                return;
            }
            let remaining = remaining.clone();
            let running = running.clone();
            let work = work.clone();
            widget.add_tick_callback(move |widget, _| {
                work(widget);
                let left = remaining.get().saturating_sub(1);
                remaining.set(left);
                if left > 0 {
                    return glib::ControlFlow::Continue;
                }
                running.set(false);
                glib::ControlFlow::Break
            });
        }
    };
    let input = gtk::EventControllerLegacy::new();
    input.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = widget.downgrade();
    let kick_for_input = kick.clone();
    input.connect_event(move |_, _| {
        if let Some(widget) = weak.upgrade() {
            kick_for_input(&widget);
        }
        glib::Propagation::Proceed
    });
    widget.add_controller(input);
    kick(widget);
    after_layout(widget, kick);
}

#[cfg(test)]
mod tests;
