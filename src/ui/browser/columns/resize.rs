// SPDX-License-Identifier: MIT

//! Column widths: dragging a column's edge, double-press autofit and the saved width.

use super::{COLUMN_TRANSITION, COLUMN_WIDTH, ColumnView};
use crate::ui::browser::ViewState;
use crate::ui::motion::{animations_enabled, emphasized_deceleration};
use crate::ui::resize_feedback::{EdgeHint, EdgePoint};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Instant;

/// How far a column's resize edge reaches either side of its line; the
/// column's scrollbar is inset by the same distance.
const RESIZE_EDGE_REACH: f64 = 6.0;

/// Two presses on one edge within this many microseconds autofit the column.
const AUTOFIT_INTERVAL_US: u64 = 400_000;

fn meets_viewport_end(state: &ViewState, shell: &gtk::Box) -> bool {
    shell.compute_bounds(&state.scroller).is_some_and(|bounds| {
        (f64::from(bounds.x() + bounds.width()) - f64::from(state.scroller.width())).abs() <= 1.0
    })
}

/// During a drag the width request leads the allocation, so the edge follows the request.
fn column_edge(shell: &gtk::Box) -> EdgePoint {
    let shell = shell.downgrade();
    Rc::new(move |overlay| {
        let shell = shell.upgrade()?;
        let bounds = shell.compute_bounds(overlay)?;
        let width = match shell.width_request() {
            requested if requested > 0 => requested as f32,
            _ => bounds.width(),
        };
        Some((bounds.x() + width, bounds.y()))
    })
}

/// The preview divider is drawn over the edge of the column that meets it.
fn preview_divider(state: &ViewState) -> Option<gtk::Widget> {
    let split = std::iter::successors(state.scroller.parent(), gtk::Widget::parent)
        .find(|widget| widget.has_css_class("preview-split"))
        .and_downcast::<gtk::Paned>()?;
    crate::ui::preview::separator(&split)
}

fn within_edge(bounds: &gtk::graphene::Rect, x: f64, y: f64) -> bool {
    let right = f64::from(bounds.x() + bounds.width());
    let across = x >= right - RESIZE_EDGE_REACH && x < right + RESIZE_EDGE_REACH;
    let along = y >= f64::from(bounds.y()) && y < f64::from(bounds.y() + bounds.height());
    across && along
}

pub(super) fn resize_edge(state: &ViewState, x: f64, y: f64) -> Option<gtk::Box> {
    state.columns.borrow().iter().find_map(|column| {
        let bounds = column.shell.compute_bounds(&state.scroller)?;
        within_edge(&bounds, x, y).then(|| column.shell.clone())
    })
}

fn set_css_class(widget: &impl IsA<gtk::Widget>, class: &str, enabled: bool) {
    if enabled {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

fn show_resize_hover(state: &ViewState, hint: &Rc<EdgeHint>, x: f64, y: f64) {
    let hovered = resize_edge(state, x, y);
    for column in state.columns.borrow().iter() {
        set_css_class(
            &column.resize_handle,
            "resize-hover",
            hovered.as_ref() == Some(&column.shell),
        );
    }
    if let Some(divider) = preview_divider(state) {
        let meets_divider = hovered
            .as_ref()
            .is_some_and(|shell| meets_viewport_end(state, shell));
        set_css_class(&divider, "resize-hover", meets_divider);
    }
    // A drag keeps its caption even where the edge stops following the pointer.
    if !state.column_resizing.get() {
        match &hovered {
            Some(shell) => hint.hover(shell, "Column width", column_edge(shell)),
            None => hint.hide(),
        }
    }
    state
        .scroller
        .set_cursor_from_name(hovered.map(|_| "col-resize"));
}

fn clear_resize_hover(state: &ViewState, hint: &EdgeHint) {
    if !state.column_resizing.get() {
        hint.hide();
    }
    for column in state.columns.borrow().iter() {
        column.resize_handle.remove_css_class("resize-hover");
    }
    if let Some(divider) = preview_divider(state) {
        divider.remove_css_class("resize-hover");
    }
    state.scroller.set_cursor(None);
}

fn install_resize_hover(state: &Rc<ViewState>, hint: &Rc<EdgeHint>) {
    let motion = gtk::EventControllerMotion::new();
    let hint_for_motion = hint.clone();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    motion.connect_motion(move |_, x, y| {
        if let Some(state) = weak.upgrade() {
            show_resize_hover(&state, &hint_for_motion, x, y);
        }
    });
    let weak = Rc::downgrade(state);
    let hint_for_leave = hint.clone();
    motion.connect_leave(move |_| {
        if let Some(state) = weak.upgrade() {
            clear_resize_hover(&state, &hint_for_leave);
        }
    });
    state.scroller.add_controller(motion);
}

struct ColumnResize {
    shell: gtk::Box,
    /// Restored if the drag is cancelled.
    requested: i32,
    initial: i32,
    start: f64,
    /// Captured once so the bound cannot creep as the preview slot yields mid-drag.
    cap: Option<i32>,
}

/// The column-resize gesture's state across its signals.
struct ResizeDrag {
    state: Weak<ViewState>,
    hint: Rc<EdgeHint>,
    active: RefCell<Option<ColumnResize>>,
    last_press: RefCell<Option<(gtk::Box, u64)>>,
}

impl ResizeDrag {
    /// Records this press and reports whether it repeats the previous one on the same edge.
    fn repeats_press(&self, shell: &gtk::Box) -> bool {
        let now = glib::monotonic_time() as u64;
        let autofit = self
            .last_press
            .borrow()
            .as_ref()
            .is_some_and(|(previous, time)| {
                *previous == *shell && now.wrapping_sub(*time) <= AUTOFIT_INTERVAL_US
            });
        *self.last_press.borrow_mut() = Some((shell.clone(), now));
        autofit
    }

    fn begin(&self, gesture: &gtk::GestureDrag, x: f64, y: f64) {
        self.active.borrow_mut().take();
        let Some(state) = self.state.upgrade() else {
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        };
        let Some(shell) = resize_edge(&state, x, y) else {
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        };
        state.column_resizing.set(true);
        let column = state
            .columns
            .borrow()
            .iter()
            .find(|c| c.shell == shell)
            .cloned();
        if let Some(column) = &column {
            column
                .animation_generation
                .set(column.animation_generation.get().saturating_add(1));
        }
        if self.repeats_press(&shell) {
            autofit_column(&state, gesture, &shell, column);
            return;
        }
        let pointer_x = gesture
            .current_event()
            .and_then(|event| event.position())
            .map_or(x, |(pointer_x, _)| pointer_x);
        *self.active.borrow_mut() = Some(ColumnResize {
            shell: shell.clone(),
            requested: shell.width_request(),
            initial: shell.width().max(COLUMN_WIDTH),
            start: pointer_x,
            cap: column_width_cap(&state),
        });
        self.hint.show(&shell, "Column width", column_edge(&shell));
        gesture.set_state(gtk::EventSequenceState::Claimed);
    }

    fn update(&self, gesture: &gtk::GestureDrag, fallback_offset_x: f64) {
        let active = self.active.borrow();
        let Some(drag) = active.as_ref() else {
            return;
        };
        let offset_x = gesture
            .current_event()
            .and_then(|event| event.position())
            .map_or(fallback_offset_x, |(current, _)| current - drag.start);
        drag.shell
            .set_size_request(resized_column_width(drag.initial, offset_x, drag.cap), -1);
        self.hint.follow();
    }

    fn end(&self) {
        self.hint.hide();
        let resized = self.active.borrow_mut().take();
        let Some(state) = self.state.upgrade() else {
            return;
        };
        state.column_resizing.set(false);
        if let Some(ColumnResize { shell, cap, .. }) = resized {
            // A press without movement sends no update, so bound the width here too.
            let width = cap_column_width(shell.width_request(), cap);
            shell.set_size_request(width, -1);
            remember_column_width(&state, width);
            state.match_column_widths(&shell);
        }
    }

    fn cancel(&self) {
        if let Some(drag) = self.active.borrow_mut().take() {
            drag.shell.set_size_request(drag.requested, -1);
        }
        self.hint.hide();
        if let Some(state) = self.state.upgrade() {
            state.column_resizing.set(false);
        }
    }
}

/// Fits the column to its widest row.
fn autofit_column(
    state: &ViewState,
    gesture: &gtk::GestureDrag,
    shell: &gtk::Box,
    column: Option<ColumnView>,
) {
    let Some(column) = column else {
        gesture.set_state(gtk::EventSequenceState::Denied);
        return;
    };
    let animation_id = column.animation_generation.get();
    let max_natural = shell
        .first_child()
        .and_downcast::<gtk::Overlay>()
        .and_then(|overlay| overlay.child())
        .map(|column| max_child_natural_width(&column))
        .unwrap_or(COLUMN_WIDTH);
    let target_width = bound_column_width(max_natural, column_width_cap(state));
    column.remember_width(target_width);
    animate_column_resize(
        &column,
        animation_id,
        shell.width().max(COLUMN_WIDTH),
        target_width,
    );
    remember_column_width(state, target_width);
    gesture.set_state(gtk::EventSequenceState::Claimed);
}

fn install_resize_drag(state: &Rc<ViewState>, hint: Rc<EdgeHint>) {
    let resize = gtk::GestureDrag::new();
    resize.set_name(Some("column-resize"));
    resize.set_button(1);
    resize.set_propagation_phase(gtk::PropagationPhase::Capture);
    let drag = Rc::new(ResizeDrag {
        state: Rc::downgrade(state),
        hint,
        active: RefCell::new(None),
        last_press: RefCell::new(None),
    });
    let drag_for_begin = drag.clone();
    resize.connect_drag_begin(move |gesture, x, y| drag_for_begin.begin(gesture, x, y));
    let drag_for_update = drag.clone();
    resize.connect_drag_update(move |gesture, fallback_offset_x, _| {
        drag_for_update.update(gesture, fallback_offset_x);
    });
    let drag_for_end = drag.clone();
    resize.connect_drag_end(move |_, _, _| drag_for_end.end());
    resize.connect_cancel(move |_, _| drag.cancel());
    state.scroller.add_controller(resize);
}

pub(in crate::ui::browser) fn install_resize_edges(state: &Rc<ViewState>) {
    let hint = Rc::new(EdgeHint::default());
    install_resize_hover(state, &hint);
    install_resize_drag(state, hint);
}

pub(super) fn remember_column_width(state: &ViewState, width: i32) {
    let preferences = crate::ui::preferences::PreferenceManager::shared();
    let width = (f64::from(width) / preferences.interface_scale()).round() as i32;
    let width = Some(width.max(COLUMN_WIDTH));
    if state.browser.is_chooser_mode() {
        preferences.set_chooser_column_width(width);
    } else {
        preferences.set_browser_column_width(width);
    }
}

pub(super) fn initial_column_width(state: &ViewState) -> i32 {
    let preferences = crate::ui::preferences::PreferenceManager::shared();
    let saved = if state.browser.is_chooser_mode() {
        preferences.chooser_column_width()
    } else {
        preferences.browser_column_width()
    };
    saved.map_or(COLUMN_WIDTH, |width| width.max(COLUMN_WIDTH))
}

/// The standard column width at the current text size, the narrowest the
/// cap may make a column.
pub(in crate::ui::browser) fn column_floor() -> i32 {
    let scale = crate::ui::preferences::PreferenceManager::shared().interface_scale();
    (f64::from(COLUMN_WIDTH) * scale).round() as i32
}

/// The visible browser area the preview layout publishes, or the columns
/// viewport while no layout is attached; none before the first allocation.
pub(super) fn column_width_cap(state: &ViewState) -> Option<i32> {
    state.column_width_limit.get().or_else(|| {
        let page = state.scroller.hadjustment().page_size();
        (page > 0.0).then(|| page.round() as i32)
    })
}

pub(super) fn cap_column_width(width: i32, cap: Option<i32>) -> i32 {
    cap.map_or(width, |cap| width.min(cap.max(column_floor())))
}

/// Pointer-chosen widths (drag, autofit) keep the 300 px minimum.
fn bound_column_width(width: i32, cap: Option<i32>) -> i32 {
    cap_column_width(width.max(COLUMN_WIDTH), cap)
}

fn resized_column_width(initial_width: i32, horizontal_offset: f64, cap: Option<i32>) -> i32 {
    bound_column_width(
        (f64::from(initial_width) + horizontal_offset).round() as i32,
        cap,
    )
}

pub(in crate::ui) fn max_child_natural_width(widget: &gtk::Widget) -> i32 {
    let (_, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
    let mut max_natural = natural;
    let mut child = widget.first_child();
    while let Some(c) = child {
        let child_max = max_child_natural_width(&c);
        if child_max > max_natural {
            max_natural = child_max;
        }
        child = c.next_sibling();
    }
    max_natural
}

/// Animates `column` to `target_width` unless a newer animation replaces `animation_id`.
fn animate_column_resize(
    column: &ColumnView,
    animation_id: u64,
    start_width: i32,
    target_width: i32,
) {
    let shell = &column.shell;
    if !animations_enabled() || (target_width - start_width).abs() < 1 {
        shell.set_size_request(target_width, -1);
        return;
    }
    let started = Instant::now();
    let shell = shell.downgrade();
    let generation = column.animation_generation.clone();
    let _tick = shell.upgrade().map(|widget| {
        widget.add_tick_callback(move |_, _| {
            if generation.get() != animation_id {
                return glib::ControlFlow::Break;
            }
            let Some(shell) = shell.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let progress =
                (started.elapsed().as_secs_f64() / COLUMN_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
            let eased = emphasized_deceleration(progress);
            let width = start_width + ((target_width - start_width) as f64 * eased).round() as i32;
            shell.set_size_request(width, -1);
            if progress >= 1.0 {
                shell.set_size_request(target_width, -1);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        })
    });
}

impl ViewState {
    fn match_column_widths(&self, resized: &gtk::Box) {
        let width = resized.width_request().max(COLUMN_WIDTH);
        for column in self.columns.borrow().iter() {
            column.remember_width(width);
            if column.shell == *resized {
                continue;
            }
            let animation_id = column.animation_generation.get().saturating_add(1);
            column.animation_generation.set(animation_id);
            animate_column_resize(
                column,
                animation_id,
                column.shell.width().max(COLUMN_WIDTH),
                width,
            );
        }
    }
}
