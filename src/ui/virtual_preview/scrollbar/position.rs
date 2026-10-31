// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gdk, glib, prelude::*};

use super::super::VirtualPreviewState;

#[derive(Clone, Copy)]
struct PublishedPosition {
    start: f64,
    progress: f64,
}

pub(super) struct DocumentPosition {
    preview: Weak<VirtualPreviewState>,
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    pixels: gtk::Adjustment,
    adjustment: gtk::Adjustment,
    coasting: Cell<bool>,
    momentum_pixels: Cell<f64>,
    momentum_update: RefCell<Option<(gdk::FrameClock, glib::SignalHandlerId)>>,
    gesture: Cell<bool>,
    direction: Cell<Option<std::cmp::Ordering>>,
    published: Cell<Option<PublishedPosition>>,
    pending: RefCell<Option<(gdk::FrameClock, glib::SignalHandlerId)>>,
}

impl DocumentPosition {
    pub(super) fn new(scroll: &gtk::ScrolledWindow, preview: &Rc<VirtualPreviewState>) -> Rc<Self> {
        let state = Rc::new(Self {
            preview: Rc::downgrade(preview),
            scroll: scroll.downgrade(),
            pixels: scroll.vadjustment(),
            adjustment: gtk::Adjustment::new(0.0, 0.0, preview.units.len() as f64, 0.1, 1.0, 0.0),
            coasting: Cell::new(false),
            momentum_pixels: Cell::new(scroll.vadjustment().value()),
            momentum_update: RefCell::new(None),
            gesture: Cell::new(false),
            direction: Cell::new(None),
            published: Cell::new(None),
            pending: RefCell::new(None),
        });
        let weak = Rc::downgrade(&state);
        state.pixels.connect_changed(move |_| {
            if let Some(state) = weak.upgrade() {
                state.stop_momentum();
                state.schedule();
            }
        });
        let weak = Rc::downgrade(&state);
        state.pixels.connect_value_changed(move |_| {
            if let Some(state) = weak.upgrade() {
                state.schedule();
            }
        });
        for controller in scroll
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
        {
            let Ok(controller) = controller.downcast::<gtk::EventControllerScroll>() else {
                continue;
            };
            let weak = Rc::downgrade(&state);
            controller.connect_decelerate(move |_, _, _| {
                if let Some(state) = weak.upgrade() {
                    state.track_momentum();
                }
            });
            let weak = Rc::downgrade(&state);
            controller.connect_scroll_begin(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.coasting.set(false);
                    state.disconnect_momentum_update();
                }
            });
        }
        let input = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&state);
        input.connect_scroll_begin(move |_| {
            if let Some(state) = weak.upgrade() {
                state.gesture.set(true);
                state.direction.set(None);
            }
        });
        let weak = Rc::downgrade(&state);
        input.connect_scroll(move |_, _, dy| {
            if let Some(state) = weak.upgrade()
                && state.gesture.get()
                && dy != 0.0
            {
                state.direction.set(Some(dy.total_cmp(&0.0)));
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&state);
        input.connect_scroll_end(move |_| {
            if let Some(state) = weak.upgrade() {
                state.gesture.set(false);
                state.direction.set(None);
            }
        });
        scroll.add_controller(input);
        if let Some(list) = preview.list.upgrade() {
            let weak = Rc::downgrade(&state);
            list.connect_map(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.schedule();
                }
            });
        }
        state
    }

    pub(super) fn adjustment(&self) -> &gtk::Adjustment {
        &self.adjustment
    }

    pub(super) fn cancel(&self) {
        self.coasting.set(false);
        self.disconnect_momentum_update();
        self.gesture.set(false);
        self.direction.set(None);
        self.published.set(None);
        if let Some((clock, handler)) = self.pending.borrow_mut().take() {
            clock.disconnect(handler);
        }
    }

    fn track_momentum(self: &Rc<Self>) {
        self.coasting.set(true);
        if self.momentum_update.borrow().is_some() {
            return;
        }
        let Some(clock) = self
            .scroll
            .upgrade()
            .and_then(|scroll| scroll.frame_clock())
        else {
            return;
        };
        self.momentum_pixels.set(self.pixels.value());
        let weak = Rc::downgrade(self);
        // Observe after GTK's native kinetic ticks but before layout can re-anchor
        // the ListView. Connecting a signal does not request additional frames.
        let handler = clock.connect_local("update", true, move |_| {
            if let Some(state) = weak.upgrade() {
                state.momentum_pixels.set(state.pixels.value());
            }
            None
        });
        self.momentum_update.replace(Some((clock, handler)));
    }

    fn disconnect_momentum_update(&self) {
        if let Some((clock, handler)) = self.momentum_update.borrow_mut().take() {
            clock.disconnect(handler);
        }
    }

    fn stop_momentum(&self) {
        if !self.coasting.replace(false) {
            return;
        }
        self.disconnect_momentum_update();
        if let Some(scroll) = self.scroll.upgrade() {
            let kinetic = scroll.is_kinetic_scrolling();
            scroll.set_kinetic_scrolling(false);
            scroll.set_kinetic_scrolling(kinetic);
            // GTK ignores anchor corrections while decelerating. Synchronize its
            // unclamped origin before the next input, without moving the document.
            self.pixels.emit_by_name::<()>("value-changed", &[]);
        }
    }

    fn schedule(self: &Rc<Self>) {
        if self.pending.borrow().is_some() {
            return;
        }
        let Some(list) = self
            .preview
            .upgrade()
            .and_then(|preview| preview.list.upgrade())
        else {
            return;
        };
        if !list.is_mapped() {
            return;
        }
        let Some(clock) = list.frame_clock() else {
            return;
        };
        // Adjustment signals precede GTK's allocation and visible-row re-anchoring.
        let weak = Rc::downgrade(self);
        let handler = clock.connect_after_paint(move |_| {
            if let Some(state) = weak.upgrade() {
                if let Some((clock, handler)) = state.pending.borrow_mut().take() {
                    clock.disconnect(handler);
                }
                state.publish();
            }
        });
        self.pending.replace(Some((clock.clone(), handler)));
        clock.request_phase(gdk::FrameClockPhase::AFTER_PAINT);
    }

    fn publish(&self) {
        // ListView truncates both the kinetic position and its restored row
        // anchor to integers. Their combined rounding is not a new pixel origin.
        if self.coasting.get() && (self.pixels.value() - self.momentum_pixels.get()).abs() >= 2.0 {
            self.stop_momentum();
        }
        let Some(preview) = self.preview.upgrade() else {
            return;
        };
        let Some(list) = preview.list.upgrade().filter(|list| list.is_mapped()) else {
            return;
        };
        let page = self.pixels.page_size();
        let Some(start) = Self::position_at(&preview, &list, 0.0) else {
            return;
        };
        let Some(end) = Self::position_at(&preview, &list, page) else {
            return;
        };
        let visible = end - start;
        let upper = preview.units.len() as f64;
        let range = upper - visible;
        let mut progress = if range > 0.0 { start / range } else { 0.0 };
        if start == 0.0 {
            progress = 0.0;
        } else if end == upper {
            progress = 1.0;
        } else if let Some(previous) = self.published.get() {
            // Resizing a trailing row must not reverse the thumb at a stationary
            // or forward-moving document position.
            progress = match self
                .direction
                .get()
                .unwrap_or_else(|| start.total_cmp(&previous.start))
            {
                std::cmp::Ordering::Greater => progress.max(previous.progress),
                std::cmp::Ordering::Less => progress.min(previous.progress),
                std::cmp::Ordering::Equal => previous.progress,
            };
        }
        self.published
            .set(Some(PublishedPosition { start, progress }));
        let scale = visible / page.max(1.0);
        self.adjustment.configure(
            progress * range,
            0.0,
            upper,
            self.pixels.step_increment() * scale,
            self.pixels.page_increment() * scale,
            visible,
        );
    }

    fn position_at(preview: &VirtualPreviewState, list: &gtk::ListView, y: f64) -> Option<f64> {
        let point = super::super::point_at(list, preview, 0.0, y)?;
        let bound = preview.bound.borrow();
        let row = bound.get(&point.unit)?;
        if !row.materialized {
            return None;
        }
        let fraction = if let Some(view) = row
            .view
            .upgrade()
            .filter(|view| view.buffer().char_count() > 0)
        {
            let bounds = view.compute_bounds(list)?;
            if y <= f64::from(bounds.y()) {
                return Some(point.unit as f64);
            }
            if y >= f64::from(bounds.y() + bounds.height()) {
                return Some(point.unit as f64 + 1.0);
            }
            let (_, y) = view.window_to_buffer_coords(
                gtk::TextWindowType::Widget,
                0,
                (y - f64::from(bounds.y())).round() as i32,
            );
            let (mut iter, _) = view.line_at_y(y);
            // GTK returns the logical line; locate its wrapped display line.
            loop {
                let mut next = iter;
                if !view.forward_display_line(&mut next)
                    || next.offset() == iter.offset()
                    || view.iter_location(&next).y() > y
                {
                    break;
                }
                iter = next;
            }
            iter.offset() as f64 / preview.units[point.unit].selection_len() as f64
        } else {
            let bounds = row.mapped_root()?.parent()?.compute_bounds(list)?;
            ((y - f64::from(bounds.y())) / f64::from(bounds.height()).max(1.0)).clamp(0.0, 1.0)
        };
        Some(point.unit as f64 + fraction)
    }
}

impl Drop for DocumentPosition {
    fn drop(&mut self) {
        if let Some((clock, handler)) = self.momentum_update.get_mut().take() {
            clock.disconnect(handler);
        }
        if let Some((clock, handler)) = self.pending.get_mut().take() {
            clock.disconnect(handler);
        }
    }
}
