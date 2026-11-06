// SPDX-License-Identifier: MIT

//! Horizontal scrolling of the column strip: wheel input and revealing a column.

use super::COLUMN_TRANSITION;
use crate::ui::browser::ViewState;
use crate::ui::motion::{animations_enabled, emphasized_deceleration};
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

fn scroll_strip(
    state: &ViewState,
    controller: &gtk::EventControllerScroll,
    dx: f64,
    dy: f64,
) -> glib::Propagation {
    let shifted = controller
        .current_event_state()
        .contains(gtk::gdk::ModifierType::SHIFT_MASK);
    let (dx, dy) = if shifted { (dy, dx) } else { (dx, dy) };
    if dx.abs() <= dy.abs() {
        return glib::Propagation::Proceed;
    }
    let adjustment = state.scroller.hadjustment();
    let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    if max <= adjustment.lower() {
        return glib::Propagation::Proceed;
    }
    let scale = match controller.unit() {
        gtk::gdk::ScrollUnit::Wheel => adjustment.page_size().powf(2.0 / 3.0),
        gtk::gdk::ScrollUnit::Surface => 2.5,
        _ => 1.0,
    };
    state
        .horizontal_scroll_generation
        .set(state.horizontal_scroll_generation.get().saturating_add(1));
    adjustment.set_value((adjustment.value() + dx * scale).clamp(adjustment.lower(), max));
    glib::Propagation::Stop
}

pub(in crate::ui::browser) fn install_horizontal_scroll(state: &Rc<ViewState>) {
    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    // Nested listing scrollers consume horizontal events even with horizontal scrolling disabled.
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_state = Rc::downgrade(state);
    controller.connect_scroll(move |controller, dx, dy| {
        let Some(state) = weak_state.upgrade() else {
            return glib::Propagation::Proceed;
        };
        scroll_strip(&state, controller, dx, dy)
    });
    state.scroller.add_controller(controller);
}

/// Scrolls the strip to `target` unless a newer scroll replaces `animation_id`.
fn animate_horizontal_scroll(
    state: &ViewState,
    adjustment: &gtk::Adjustment,
    target: f64,
    animation_id: u64,
) {
    let start = adjustment.value();
    if !animations_enabled() || (target - start).abs() < 0.5 {
        adjustment.set_value(target);
        return;
    }

    let started = Instant::now();
    let adjustment = adjustment.clone();
    let generation = state.horizontal_scroll_generation.clone();
    let _tick = state.scroller.add_tick_callback(move |_, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }
        let progress =
            (started.elapsed().as_secs_f64() / COLUMN_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_deceleration(progress);
        adjustment.set_value(start + (target - start) * eased);
        if progress >= 1.0 {
            adjustment.set_value(target);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

impl ViewState {
    /// Trailing columns borrow preview space without displacing the active column.
    fn reveal_anchor(&self, shell: gtk::Box) -> gtk::Box {
        let columns = self.columns.borrow();
        let depth = columns.iter().position(|column| column.shell == shell);
        match (depth, self.browser.active_depth()) {
            (Some(depth), Some(active)) if depth > active => columns
                .get(active)
                .map_or(shell, |column| column.shell.clone()),
            _ => shell,
        }
    }

    pub(in crate::ui::browser) fn reveal_column(self: &Rc<Self>, shell: gtk::Box) {
        let shell = self.reveal_anchor(shell);
        let animation_id = self.horizontal_scroll_generation.get().saturating_add(1);
        self.horizontal_scroll_generation.set(animation_id);
        self.columns_widget.set_margin_end(0);
        let weak = Rc::downgrade(self);
        let measured_shell = shell.downgrade();
        // Wait for the preview slot to resize before measuring the viewport.
        let laid_out = Cell::new(false);
        let _tick = self.scroller.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(measured_shell) = measured_shell.upgrade() else {
                return glib::ControlFlow::Break;
            };
            state.reveal_measured_column(&measured_shell, animation_id, &laid_out)
        });
    }

    fn reveal_measured_column(
        &self,
        measured_shell: &gtk::Box,
        animation_id: u64,
        laid_out: &Cell<bool>,
    ) -> glib::ControlFlow {
        let superseded = self.horizontal_scroll_generation.get() != animation_id;
        if superseded || measured_shell.parent().is_none() {
            return glib::ControlFlow::Break;
        }
        let adjustment = self.scroller.hadjustment();
        let first_frame = !laid_out.replace(true);
        let unmeasured = measured_shell.width() <= 0 || adjustment.page_size() <= 0.0;
        if first_frame || unmeasured {
            return glib::ControlFlow::Continue;
        }
        let depth = self
            .columns
            .borrow()
            .iter()
            .position(|column| column.shell == *measured_shell);
        let Some(span) = depth.and_then(|depth| self.column_span(depth)) else {
            return glib::ControlFlow::Break;
        };
        let target = span.reveal_target(
            adjustment.value(),
            adjustment.page_size(),
            adjustment.lower(),
            adjustment.upper(),
        );
        animate_horizontal_scroll(self, &adjustment, target, animation_id);
        glib::ControlFlow::Break
    }
}
