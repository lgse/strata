// SPDX-License-Identifier: MIT

//! Hover tooltips for entry rows and icon cards.
//!
//! GTK's own hover delay drops to 60 ms once any tooltip is visible, which would
//! flash popovers while browsing, so the pointer must rest first.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

pub(crate) const REST_DELAY: Duration = Duration::from_millis(900);
const REST_SLOP_PX: f64 = 4.0;

#[derive(Default)]
pub(crate) struct EntryTooltip {
    rest: Cell<Option<(Instant, f64, f64)>>,
    requery: RefCell<Option<glib::SourceId>>,
}

impl EntryTooltip {
    /// Returns whether the pointer moved far enough to restart the rest delay.
    pub(crate) fn pointer_moved(&self, x: f64, y: f64, now: Instant) -> bool {
        if self
            .rest
            .get()
            .is_some_and(|(_, rest_x, rest_y)| within_slop(x - rest_x, y - rest_y))
        {
            return false;
        }
        self.cancel_requery();
        self.rest.set(Some((now, x, y)));
        true
    }

    pub(crate) fn reset(&self) {
        self.cancel_requery();
        self.rest.set(None);
    }

    pub(crate) fn is_rested(&self, now: Instant) -> bool {
        self.rest
            .get()
            .is_some_and(|(since, ..)| now.saturating_duration_since(since) >= REST_DELAY)
    }

    fn cancel_requery(&self) {
        if let Some(source) = self.requery.take() {
            source.remove();
        }
    }

    pub(crate) fn install<W, F>(widget: &W, provide_text: F) -> Rc<Self>
    where
        W: IsA<gtk::Widget>,
        F: Fn(&W, f64, f64) -> Option<String> + 'static,
    {
        let state = Rc::new(Self::default());
        widget.set_has_tooltip(true);

        let motion = gtk::EventControllerMotion::new();
        let moved = {
            let state = state.clone();
            move |controller: &gtk::EventControllerMotion, x: f64, y: f64| {
                let Some(w) = controller.widget().and_downcast::<W>() else {
                    return;
                };
                if state.pointer_moved(x, y, Instant::now()) {
                    schedule_requery(&state, &w);
                }
            }
        };
        motion.connect_enter(moved.clone());
        motion.connect_motion(moved);
        let left = state.clone();
        motion.connect_leave(move |_| left.reset());
        widget.add_controller(motion);

        let press = gtk::GestureClick::builder()
            .button(0)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let pressed = state.clone();
        press.connect_pressed(move |_, _, _, _| pressed.reset());
        widget.add_controller(press);

        let query_state = state.clone();
        widget.connect_query_tooltip(move |w, x, y, keyboard_mode, tooltip| {
            if keyboard_mode {
                return false;
            }
            if !query_state.is_rested(Instant::now()) {
                return false;
            }
            let Some(text) = provide_text(w, f64::from(x), f64::from(y)) else {
                return false;
            };
            tooltip.set_text(Some(&text));
            true
        });

        state
    }
}

pub(crate) fn name_text(name: &str, truncated: bool, date: Option<&str>) -> Option<String> {
    match date.filter(|date| !date.is_empty()) {
        Some(date) => Some(format!("{name}\n{date}")),
        None if truncated => Some(name.to_owned()),
        None => None,
    }
}

fn within_slop(dx: f64, dy: f64) -> bool {
    dx.abs() <= REST_SLOP_PX && dy.abs() <= REST_SLOP_PX
}

fn schedule_requery<W: IsA<gtk::Widget>>(state: &Rc<EntryTooltip>, widget: &W) {
    let pending = state.clone();
    let target = widget.downgrade();
    let source = glib::timeout_add_local_once(REST_DELAY, move || {
        pending.requery.take();
        if let Some(target) = target.upgrade() {
            target.trigger_tooltip_query();
        }
    });
    state.requery.replace(Some(source));
}

#[cfg(test)]
mod tests;
