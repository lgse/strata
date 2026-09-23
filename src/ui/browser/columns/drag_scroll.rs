// SPDX-License-Identifier: MIT

use crate::ui::browser::ViewState;
use crate::ui::scrolling::advance;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

const EDGE_MARGIN: f64 = 44.0;
const EDGE_SATURATION: f64 = 16.0;
const MAX_HORIZONTAL_SPEED: f64 = 720.0;
const MAX_VERTICAL_SPEED: f64 = 520.0;
const RAMP: Duration = Duration::from_millis(350);
const FRAME: Duration = Duration::from_millis(16);
const MAX_FRAME: Duration = Duration::from_millis(50);

// Scroller-relative coordinates stay fixed as content moves beneath the drag.
pub(in crate::ui::browser) struct DragAutoscroll {
    state: Weak<ViewState>,
    controller: glib::WeakRef<gtk::DropControllerMotion>,
    pointer: Cell<(f64, f64)>,
    burst_x: Cell<Option<(Instant, i8)>>,
    burst_y: Cell<Option<(Instant, i8)>>,
    frame_time: Cell<i64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

impl ViewState {
    // Blank strip space has no drop target, so track motion on the scroller.
    pub(in crate::ui::browser) fn install_drag_autoscroll(self: &Rc<Self>) {
        let controller = gtk::DropControllerMotion::new();
        let tracker = Rc::new(DragAutoscroll {
            state: Rc::downgrade(self),
            controller: controller.downgrade(),
            pointer: Cell::new((0.0, 0.0)),
            burst_x: Cell::new(None),
            burst_y: Cell::new(None),
            frame_time: Cell::new(0),
            tick: RefCell::new(None),
        });
        self.drag_autoscroll.replace(Some(tracker.clone()));
        let tracker_for_enter = tracker.clone();
        controller.connect_enter(move |_, x, y| tracker_for_enter.track(x, y));
        let tracker_for_motion = tracker.clone();
        controller.connect_motion(move |_, x, y| tracker_for_motion.track(x, y));
        let tracker_for_leave = tracker.clone();
        controller.connect_leave(move |_| tracker_for_leave.stop());
        let tracker_for_unmap = tracker;
        self.scroller
            .connect_unmap(move |_| tracker_for_unmap.stop());
        self.scroller.add_controller(controller);
    }
}

impl DragAutoscroll {
    fn track(self: &Rc<Self>, x: f64, y: f64) {
        self.pointer.set((x, y));
        if self.file_drag_over() && self.should_scroll() {
            self.start_ticks();
        } else {
            self.stop();
        }
    }

    fn file_drag_over(&self) -> bool {
        let Some(controller) = self.controller.upgrade() else {
            return false;
        };
        controller.contains_pointer()
            && controller.drop().is_some_and(|drop| {
                let formats = drop.formats();
                formats.contains_type(gdk::FileList::static_type())
                    || formats.contain_mime_type("text/uri-list")
            })
    }

    fn should_scroll(&self) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        let (x, y) = self.pointer.get();
        let scroller = &state.scroller;
        let horizontal = scroller.hadjustment();
        let direction_x = edge_direction(x, f64::from(scroller.width()));
        if direction_x != 0.0 && advanceable(&horizontal, direction_x) {
            return true;
        }
        let columns = state.columns.borrow();
        columns.iter().any(|column| {
            let Some(shell) = column.shell.compute_bounds(scroller) else {
                return false;
            };
            if x < f64::from(shell.x()) || x >= f64::from(shell.x() + shell.width()) {
                return false;
            }
            let Some(listing) = column.listing_scroll.compute_bounds(scroller) else {
                return false;
            };
            let direction =
                listing_band_direction(y, f64::from(listing.y()), f64::from(listing.height()));
            direction != 0.0 && advanceable(&column.listing_scroll.vadjustment(), direction)
        })
    }

    fn apply_scroll(&self, dt: Duration) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        let (x, y) = self.pointer.get();
        let scroller = &state.scroller;
        let horizontal = scroller.hadjustment();
        let direction_x = edge_direction(x, f64::from(scroller.width()));
        let direction_x = direction_x * f64::from(advanceable(&horizontal, direction_x));
        let vertical = {
            let columns = state.columns.borrow();
            columns.iter().find_map(|column| {
                let shell = column.shell.compute_bounds(scroller)?;
                if x < f64::from(shell.x()) || x >= f64::from(shell.x() + shell.width()) {
                    return None;
                }
                let listing = column.listing_scroll.compute_bounds(scroller)?;
                let direction =
                    listing_band_direction(y, f64::from(listing.y()), f64::from(listing.height()));
                let adjustment = column.listing_scroll.vadjustment();
                advanceable(&adjustment, direction).then_some((adjustment, direction))
            })
        };
        if direction_x == 0.0 && vertical.is_none() {
            self.burst_x.set(None);
            self.burst_y.set(None);
            return false;
        }

        let seconds = dt.as_secs_f64();
        if direction_x != 0.0 {
            let sign_x = direction_x.signum() as i8;
            let dwell_x = match self.burst_x.get() {
                Some((start, s)) if s == sign_x => start.elapsed(),
                _ => {
                    self.burst_x.set(Some((Instant::now(), sign_x)));
                    Duration::ZERO
                }
            };
            advance(
                &horizontal,
                scroll_speed(direction_x, dwell_x, MAX_HORIZONTAL_SPEED) * seconds,
            );
        } else {
            self.burst_x.set(None);
        }

        if let Some((adjustment, direction_y)) = vertical {
            let sign_y = direction_y.signum() as i8;
            let dwell_y = match self.burst_y.get() {
                Some((start, s)) if s == sign_y => start.elapsed(),
                _ => {
                    self.burst_y.set(Some((Instant::now(), sign_y)));
                    Duration::ZERO
                }
            };
            advance(
                &adjustment,
                scroll_speed(direction_y, dwell_y, MAX_VERTICAL_SPEED) * seconds,
            );
        } else {
            self.burst_y.set(None);
        }

        true
    }

    fn start_ticks(self: &Rc<Self>) {
        if self.tick.borrow().is_some() {
            return;
        }
        let Some(state) = self.state.upgrade() else {
            return;
        };
        state
            .horizontal_scroll_generation
            .set(state.horizontal_scroll_generation.get().saturating_add(1));
        self.frame_time.set(0);
        let tracker = self.clone();
        let id = state.scroller.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let previous = tracker.frame_time.replace(now);
            let dt = if previous <= 0 || now <= previous {
                FRAME
            } else {
                Duration::from_micros((now - previous) as u64).min(MAX_FRAME)
            };
            if tracker.file_drag_over() && tracker.apply_scroll(dt) {
                return glib::ControlFlow::Continue;
            }
            tracker.tick.borrow_mut().take();
            glib::ControlFlow::Break
        });
        self.tick.replace(Some(id));
    }

    pub(in crate::ui::browser) fn stop(&self) {
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        self.burst_x.set(None);
        self.burst_y.set(None);
    }
}

fn edge_direction(position: f64, size: f64) -> f64 {
    if size <= EDGE_MARGIN * 2.0 {
        return 0.0;
    }
    if position < EDGE_MARGIN {
        let dist = position.max(0.0);
        let p = if dist <= EDGE_SATURATION {
            1.0
        } else {
            1.0 - (dist - EDGE_SATURATION) / (EDGE_MARGIN - EDGE_SATURATION)
        };
        return -p.clamp(0.0, 1.0);
    }
    if position > size - EDGE_MARGIN {
        let dist = (size - position).max(0.0);
        let p = if dist <= EDGE_SATURATION {
            1.0
        } else {
            1.0 - (dist - EDGE_SATURATION) / (EDGE_MARGIN - EDGE_SATURATION)
        };
        return p.clamp(0.0, 1.0);
    }
    0.0
}

pub(super) fn listing_band_direction(y: f64, band_top: f64, band_height: f64) -> f64 {
    if y < band_top || y >= band_top + band_height {
        return 0.0;
    }
    edge_direction(y - band_top, band_height)
}

pub(super) fn scroll_speed(direction: f64, dwell: Duration, max_speed: f64) -> f64 {
    let proximity = direction.abs().clamp(0.0, 1.0);
    if proximity <= 0.0 {
        return 0.0;
    }
    let ramp = (dwell.as_secs_f64() / RAMP.as_secs_f64()).clamp(0.0, 1.0);
    let dwell_factor = 0.35 + 0.65 * (3.0 * ramp * ramp - 2.0 * ramp * ramp * ramp);
    let speed = max_speed * proximity * proximity * dwell_factor;
    direction.signum() * speed
}

fn advanceable(adjustment: &gtk::Adjustment, direction: f64) -> bool {
    if direction < 0.0 {
        adjustment.value() > adjustment.lower()
    } else if direction > 0.0 {
        adjustment.value() < adjustment.upper() - adjustment.page_size()
    } else {
        false
    }
}
