// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use super::highlight::{selection_text, update_all_bound_selection};
use super::hit_test::{link_info_at, matching_link, point_at, update_link_hover};
use super::{AUTOSCROLL_EDGE, SelectionPoint, VirtualPreviewState, selection};
use crate::services::has_web_scheme;

pub(super) fn install_pointer_selection(
    list: &gtk::ListView,
    scroll: &gtk::ScrolledWindow,
    state: Rc<VirtualPreviewState>,
) {
    let drag_claimed = Rc::new(Cell::new(false));
    let drag_allowed = Rc::new(Cell::new(false));
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_list = list.downgrade();
    let state_for_press = state.clone();
    let drag_claimed_for_press = drag_claimed.clone();
    click.connect_pressed(move |gesture, presses, x, y| {
        let Some(list) = weak_list.upgrade() else {
            return;
        };
        state_for_press.pressed_link.borrow_mut().take();
        if point_hits_class(&list, x, y, "preview-code-copy")
            || point_hits_class(&list, x, y, "preview-table-interactive")
        {
            return;
        }
        let extend = gesture
            .current_event_state()
            .contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if presses == 1 && !extend {
            state_for_press
                .pressed_link
                .replace(link_info_at(&list, &state_for_press, x, y).map(|(_, _, uri)| uri));
        }
        drag_claimed_for_press.set(false);
        list.grab_focus();
        let point = point_at(&list, &state_for_press, x, y)
            .unwrap_or(SelectionPoint { unit: 0, offset: 0 });
        state_for_press
            .selection
            .set(Some(selection::clicked_selection(
                &state_for_press.units,
                state_for_press.selection.get(),
                point,
                presses,
                extend,
            )));
        update_all_bound_selection(&state_for_press);
    });

    let weak_list = list.downgrade();
    let state_for_release = state.clone();
    let drag_claimed_for_release = drag_claimed.clone();
    click.connect_released(move |_, _, x, y| {
        let pressed = state_for_release.pressed_link.borrow_mut().take();
        if drag_claimed_for_release.replace(false) {
            return;
        }
        let Some(list) = weak_list.upgrade() else {
            return;
        };
        let released = link_info_at(&list, &state_for_release, x, y).map(|(_, _, uri)| uri);
        if let Some(uri) = matching_link(pressed.as_deref(), released.as_deref())
            && has_web_scheme(uri)
        {
            open_web_link(uri, &list);
        }
    });
    list.add_controller(click);

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_list = list.downgrade();
    let state_for_begin = state.clone();
    let drag_allowed_for_begin = drag_allowed.clone();
    drag.connect_drag_begin(move |_, x, y| {
        let Some(list) = weak_list.upgrade() else {
            return;
        };
        let allowed = !point_hits_class(&list, x, y, "preview-code-copy")
            && !point_hits_class(&list, x, y, "preview-table-interactive");
        drag_allowed_for_begin.set(allowed);
        if !allowed {
            return;
        }
        state_for_begin.press.set((x, y));
        state_for_begin.pointer.set((x, y));
    });

    let weak_list = list.downgrade();
    let weak_scroll = scroll.downgrade();
    let state_for_update = state.clone();
    let drag_allowed_for_update = drag_allowed.clone();
    let drag_claimed_for_update = drag_claimed.clone();
    drag.connect_drag_update(move |gesture, offset_x, offset_y| {
        let Some(list) = weak_list.upgrade() else {
            return;
        };
        let (start_x, start_y) = state_for_update.press.get();
        let (x, y) = (start_x + offset_x, start_y + offset_y);
        if !state_for_update.dragging.get() {
            if !drag_allowed_for_update.get()
                || !drag_threshold_crossed(&list, start_x, start_y, x, y)
            {
                return;
            }
            drag_claimed_for_update.set(true);
            gesture.set_state(gtk::EventSequenceState::Claimed);
            state_for_update.pressed_link.borrow_mut().take();
            list.grab_focus();
            state_for_update.dragging.set(true);
            state_for_update
                .drag_generation
                .set(state_for_update.drag_generation.get().wrapping_add(1));
            update_all_bound_selection(&state_for_update);
            if let Some(scroll) = weak_scroll.upgrade() {
                start_autoscroll(&list, &scroll, &state_for_update);
            }
        }
        state_for_update.pointer.set((x, y));
        update_drag_focus(&list, &state_for_update, x, y);
    });

    let weak_list = list.downgrade();
    let state_for_end = state.clone();
    let drag_allowed_for_end = drag_allowed.clone();
    drag.connect_drag_end(move |_, offset_x, offset_y| {
        drag_allowed_for_end.set(false);
        if !state_for_end.dragging.get() {
            return;
        }
        let Some(list) = weak_list.upgrade() else {
            return;
        };
        let (start_x, start_y) = state_for_end.press.get();
        let (x, y) = (start_x + offset_x, start_y + offset_y);
        update_drag_focus(&list, &state_for_end, x, y);
        if let Some(text) = selection_text(&state_for_end) {
            list.primary_clipboard().set_text(&text);
        }
        state_for_end.dragging.set(false);
    });

    let state_for_cancel = state.clone();
    let drag_allowed_for_cancel = drag_allowed;
    drag.connect_cancel(move |_, _| {
        drag_allowed_for_cancel.set(false);
        state_for_cancel.pressed_link.borrow_mut().take();
        state_for_cancel.dragging.set(false);
        state_for_cancel
            .drag_generation
            .set(state_for_cancel.drag_generation.get().wrapping_add(1));
    });
    list.add_controller(drag);

    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_list = list.downgrade();
    let state_for_motion = state.clone();
    motion.connect_motion(move |_, x, y| {
        if state_for_motion.dragging.get() {
            return;
        }
        let Some(list) = weak_list.upgrade() else {
            return;
        };
        let next = link_info_at(&list, &state_for_motion, x, y).map(|(unit, span, _)| (unit, span));
        update_link_hover(&state_for_motion, next);
        list.set_cursor_from_name(Some(if next.is_some() { "pointer" } else { "text" }));
    });
    let weak_list = list.downgrade();
    motion.connect_leave(move |_| {
        update_link_hover(&state, None);
        if let Some(list) = weak_list.upgrade() {
            list.set_cursor_from_name(Some("text"));
        }
    });
    list.add_controller(motion);
}

pub(super) fn drag_threshold_crossed(
    widget: &impl IsA<gtk::Widget>,
    start_x: f64,
    start_y: f64,
    current_x: f64,
    current_y: f64,
) -> bool {
    widget.drag_check_threshold(
        start_x.round() as i32,
        start_y.round() as i32,
        current_x.round() as i32,
        current_y.round() as i32,
    )
}

pub(super) fn point_hits_class(list: &gtk::ListView, x: f64, y: f64, class: &str) -> bool {
    let mut target = list.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(widget) = target {
        if widget.has_css_class(class) {
            return true;
        }
        target = widget.parent();
    }
    false
}

pub(super) fn start_autoscroll(
    list: &gtk::ListView,
    scroll: &gtk::ScrolledWindow,
    state: &Rc<VirtualPreviewState>,
) {
    let expected = state.drag_generation.get();
    let weak_state = Rc::downgrade(state);
    let weak_list = list.downgrade();
    let weak_scroll = scroll.downgrade();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(state) = weak_state
            .upgrade()
            .filter(|state| state.dragging.get() && state.drag_generation.get() == expected)
        else {
            return glib::ControlFlow::Break;
        };
        let Some(list) = weak_list.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let Some(scroll) = weak_scroll.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let (_, y) = state.pointer.get();
        let height = f64::from(list.height());
        let delta = if y < AUTOSCROLL_EDGE {
            -((AUTOSCROLL_EDGE - y) * 0.35).min(24.0)
        } else if y > height - AUTOSCROLL_EDGE {
            ((y - (height - AUTOSCROLL_EDGE)) * 0.35).min(24.0)
        } else {
            0.0
        };
        if delta != 0.0 {
            let adjustment = scroll.vadjustment();
            let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            adjustment.set_value((adjustment.value() + delta).clamp(adjustment.lower(), upper));
            let (x, y) = state.pointer.get();
            update_drag_focus(&list, &state, x, y);
        }
        glib::ControlFlow::Continue
    });
}

pub(super) fn update_drag_focus(list: &gtk::ListView, state: &VirtualPreviewState, x: f64, y: f64) {
    let Some(point) = point_at(list, state, x, y) else {
        return;
    };
    if let Some(mut selection) = state.selection.get() {
        selection.focus = point;
        state.selection.set(Some(selection));
        update_all_bound_selection(state);
    }
}

pub(in crate::ui) fn open_web_link(uri: &str, parent: &impl IsA<gtk::Widget>) {
    let context = crate::child_environment::launch_context(&parent.display());
    if let Err(error) = gio::AppInfo::launch_default_for_uri(uri, Some(&context)) {
        super::super::modal::show_error_dialog(
            parent,
            &crate::i18n::tr("Unable to open link"),
            &error.to_string(),
        );
    }
}
