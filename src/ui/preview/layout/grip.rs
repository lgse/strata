// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::super::PreviewState;
use super::geometry::separator;

/// Divider position and pointer x where a grip drag began.
type GripOrigin = Rc<Cell<Option<(i32, f64)>>>;

/// Moves the divider from the grip, measuring in window coordinates because the
/// grip itself moves with the divider.
pub(super) fn install_resize_grip(split: &gtk::Paned, state: &Rc<PreviewState>) {
    // A hidden preview has no width to set, so only a docked one offers a resize.
    let docked_split = split.downgrade();
    let grip = state.resize_grip.clone();
    let hint = state.sizing.grip_hint.clone();
    let follow_reveal = move |revealer: &gtk::Revealer| {
        let docked = revealer.reveals_child();
        grip.set_visible(docked);
        if !docked {
            hint.hide();
        }
        if let Some(handle) = docked_split.upgrade().and_then(|split| separator(&split)) {
            // GTK does not pick a divider without a cursor, so it cannot be dragged.
            handle.set_cursor_from_name(docked.then_some("col-resize"));
        }
    };
    follow_reveal(&state.revealer);
    state.revealer.connect_reveal_child_notify(follow_reveal);

    install_edge_hover(split, state);

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    let origin: GripOrigin = Rc::new(Cell::new(None));
    let weak = Rc::downgrade(state);
    let begun_split = split.downgrade();
    let begun = origin.clone();
    drag.connect_drag_begin(move |gesture, _, _| {
        begin_grip_drag(gesture, &weak, &begun_split, &begun);
    });
    let moved_split = split.downgrade();
    let moved = origin.clone();
    drag.connect_drag_update(move |gesture, _, _| {
        let (Some(split), Some((start, pointer_start))) = (moved_split.upgrade(), moved.get())
        else {
            return;
        };
        if let Some((pointer_x, _)) = gesture.current_event().and_then(|event| event.position()) {
            split.set_position(start + (pointer_x - pointer_start).round() as i32);
        }
    });
    let weak = Rc::downgrade(state);
    let ended = origin.clone();
    drag.connect_drag_end(move |_, _, _| end_grip_drag(&ended, &weak));
    let weak = Rc::downgrade(state);
    drag.connect_cancel(move |_, _| end_grip_drag(&origin, &weak));
    state.resize_grip.add_controller(drag);
}

fn begin_grip_drag(
    gesture: &gtk::GestureDrag,
    state: &Weak<PreviewState>,
    split: &glib::WeakRef<gtk::Paned>,
    origin: &GripOrigin,
) {
    let pointer = gesture.current_event().and_then(|event| event.position());
    let (Some(state), Some(split), Some((pointer_x, _))) =
        (state.upgrade(), split.upgrade(), pointer)
    else {
        gesture.set_state(gtk::EventSequenceState::Denied);
        return;
    };
    if !state.can_drag_divider() {
        gesture.set_state(gtk::EventSequenceState::Denied);
        return;
    }
    gesture.set_state(gtk::EventSequenceState::Claimed);
    state.begin_divider_drag(&split);
    origin.set(Some((split.position(), pointer_x)));
}

fn end_grip_drag(origin: &GripOrigin, state: &Weak<PreviewState>) {
    if origin.take().is_some()
        && let Some(state) = state.upgrade()
    {
        state.end_divider_drag();
    }
}

/// The divider line also drags the divider, so it shares the grip's hover: each
/// lights the other, and both show one caption.
fn install_edge_hover(split: &gtk::Paned, state: &Rc<PreviewState>) {
    let Some(divider) = separator(split) else {
        return;
    };
    let hovers = [
        gtk::EventControllerMotion::new(),
        gtk::EventControllerMotion::new(),
    ];
    let watched: Rc<[glib::WeakRef<gtk::EventControllerMotion>]> =
        hovers.iter().map(|hover| hover.downgrade()).collect();
    let partners = [
        state.resize_grip.clone().upcast::<gtk::Widget>(),
        divider.clone(),
    ];
    for (hover, partner) in hovers.iter().zip(partners) {
        let partner_for_enter = partner.downgrade();
        let hovered = Rc::downgrade(state);
        hover.connect_enter(move |_, _, _| {
            if let Some(state) = hovered.upgrade() {
                enter_edge(&state, &partner_for_enter);
            }
        });
        let partner = partner.downgrade();
        let left = Rc::downgrade(state);
        let watched = watched.clone();
        hover.connect_leave(move |_| {
            if let Some(partner) = partner.upgrade() {
                partner.remove_css_class("resize-hover");
            }
            hide_hint_off_edge(left.clone(), watched.clone());
        });
    }
    let [divider_hover, grip_hover] = hovers;
    divider.add_controller(divider_hover);
    state.resize_grip.add_controller(grip_hover);
}

fn enter_edge(state: &PreviewState, partner: &glib::WeakRef<gtk::Widget>) {
    if !state.resize_grip.is_visible() {
        return;
    }
    if let Some(partner) = partner.upgrade() {
        partner.add_css_class("resize-hover");
    }
    let grip = state.resize_grip.downgrade();
    state.sizing.grip_hint.hover(
        &state.resize_grip,
        "Preview panel minimum width",
        Rc::new(move |overlay| {
            let bounds = grip.upgrade()?.compute_bounds(overlay)?;
            Some((bounds.x(), bounds.y()))
        }),
    );
}

/// Crossing between the line and the grip leaves one before entering the other.
fn hide_hint_off_edge(
    state: Weak<PreviewState>,
    watched: Rc<[glib::WeakRef<gtk::EventControllerMotion>]>,
) {
    glib::idle_add_local_once(move || {
        let still_on_edge = watched.iter().any(|hover| {
            hover
                .upgrade()
                .is_some_and(|hover| hover.contains_pointer())
        });
        if still_on_edge {
            return;
        }
        if let Some(state) = state.upgrade()
            && !state.sizing.dragging.get()
        {
            state.sizing.grip_hint.hide();
        }
    });
}
