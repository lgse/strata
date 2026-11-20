// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use super::highlight::{selection_text, update_all_bound_selection};
use super::hit_test::{link_info_at, matching_link, point_at, update_link_hover};
use super::{AUTOSCROLL_EDGE, SelectionPoint, VirtualPreviewState, selection};
use crate::services::has_web_scheme;

/// Pointer selection, link activation and hover for one virtual preview list.
struct PointerSelection {
    list: glib::WeakRef<gtk::ListView>,
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    state: Rc<VirtualPreviewState>,
    drag_claimed: Cell<bool>,
    drag_allowed: Cell<bool>,
}

pub(super) fn install_pointer_selection(
    list: &gtk::ListView,
    scroll: &gtk::ScrolledWindow,
    state: Rc<VirtualPreviewState>,
) {
    let pointer = Rc::new(PointerSelection {
        list: list.downgrade(),
        scroll: scroll.downgrade(),
        state,
        drag_claimed: Cell::new(false),
        drag_allowed: Cell::new(false),
    });
    list.add_controller(click_controller(&pointer));
    list.add_controller(drag_controller(&pointer));
    list.add_controller(motion_controller(&pointer));
}

fn click_controller(pointer: &Rc<PointerSelection>) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let pointer_for_press = pointer.clone();
    click.connect_pressed(move |gesture, presses, x, y| {
        pointer_for_press.pressed(gesture, presses, (x, y));
    });
    let pointer = pointer.clone();
    click.connect_released(move |_, _, x, y| pointer.released((x, y)));
    click
}

fn drag_controller(pointer: &Rc<PointerSelection>) -> gtk::GestureDrag {
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let pointer_for_begin = pointer.clone();
    drag.connect_drag_begin(move |_, x, y| pointer_for_begin.drag_begin((x, y)));
    let pointer_for_update = pointer.clone();
    drag.connect_drag_update(move |gesture, offset_x, offset_y| {
        pointer_for_update.drag_update(gesture, (offset_x, offset_y));
    });
    let pointer_for_end = pointer.clone();
    drag.connect_drag_end(move |_, offset_x, offset_y| {
        pointer_for_end.drag_end((offset_x, offset_y));
    });
    let pointer = pointer.clone();
    drag.connect_cancel(move |_, _| pointer.drag_cancel());
    drag
}

fn motion_controller(pointer: &Rc<PointerSelection>) -> gtk::EventControllerMotion {
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let pointer_for_motion = pointer.clone();
    motion.connect_motion(move |_, x, y| pointer_for_motion.hover((x, y)));
    let pointer = pointer.clone();
    motion.connect_leave(move |_| pointer.leave());
    motion
}

/// Code copy buttons and interactive tables keep their own presses.
fn hits_row_control(list: &gtk::ListView, x: f64, y: f64) -> bool {
    point_hits_class(list, x, y, "preview-code-copy")
        || point_hits_class(list, x, y, "preview-table-interactive")
}

impl PointerSelection {
    fn pressed(&self, gesture: &gtk::GestureClick, presses: i32, (x, y): (f64, f64)) {
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let state = &self.state;
        state.pressed_link.borrow_mut().take();
        if hits_row_control(&list, x, y) {
            return;
        }
        let extend = gesture
            .current_event_state()
            .contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if presses == 1 && !extend {
            state
                .pressed_link
                .replace(link_info_at(&list, state, x, y).map(|(_, _, uri)| uri));
        }
        self.drag_claimed.set(false);
        list.grab_focus();
        let point = point_at(&list, state, x, y).unwrap_or(SelectionPoint { unit: 0, offset: 0 });
        state.selection.set(Some(selection::clicked_selection(
            &state.units,
            state.selection.get(),
            point,
            presses,
            extend,
        )));
        update_all_bound_selection(state);
    }

    fn released(&self, (x, y): (f64, f64)) {
        let pressed = self.state.pressed_link.borrow_mut().take();
        if self.drag_claimed.replace(false) {
            return;
        }
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let released = link_info_at(&list, &self.state, x, y).map(|(_, _, uri)| uri);
        if let Some(uri) = matching_link(pressed.as_deref(), released.as_deref())
            && has_web_scheme(uri)
        {
            open_web_link(uri, &list);
        }
    }

    fn drag_begin(&self, (x, y): (f64, f64)) {
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let allowed = !hits_row_control(&list, x, y);
        self.drag_allowed.set(allowed);
        if !allowed {
            return;
        }
        self.state.press.set((x, y));
        self.state.pointer.set((x, y));
    }

    fn drag_update(&self, gesture: &gtk::GestureDrag, offset: (f64, f64)) {
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let start = self.state.press.get();
        let (x, y) = (start.0 + offset.0, start.1 + offset.1);
        if !self.state.dragging.get() {
            if !self.drag_allowed.get() || !drag_threshold_crossed(&list, start.0, start.1, x, y) {
                return;
            }
            self.start_drag(&list, gesture);
        }
        self.state.pointer.set((x, y));
        update_drag_focus(&list, &self.state, x, y);
    }

    fn start_drag(&self, list: &gtk::ListView, gesture: &gtk::GestureDrag) {
        let state = &self.state;
        self.drag_claimed.set(true);
        gesture.set_state(gtk::EventSequenceState::Claimed);
        state.pressed_link.borrow_mut().take();
        list.grab_focus();
        state.dragging.set(true);
        state
            .drag_generation
            .set(state.drag_generation.get().wrapping_add(1));
        update_all_bound_selection(state);
        if let Some(scroll) = self.scroll.upgrade() {
            start_autoscroll(list, &scroll, state);
        }
    }

    fn drag_end(&self, offset: (f64, f64)) {
        self.drag_allowed.set(false);
        if !self.state.dragging.get() {
            return;
        }
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let start = self.state.press.get();
        update_drag_focus(&list, &self.state, start.0 + offset.0, start.1 + offset.1);
        if let Some(text) = selection_text(&self.state) {
            list.primary_clipboard().set_text(&text);
        }
        self.state.dragging.set(false);
    }

    fn drag_cancel(&self) {
        let state = &self.state;
        self.drag_allowed.set(false);
        state.pressed_link.borrow_mut().take();
        state.dragging.set(false);
        state
            .drag_generation
            .set(state.drag_generation.get().wrapping_add(1));
    }

    fn hover(&self, (x, y): (f64, f64)) {
        if self.state.dragging.get() {
            return;
        }
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let next = link_info_at(&list, &self.state, x, y).map(|(unit, span, _)| (unit, span));
        update_link_hover(&self.state, next);
        list.set_cursor_from_name(Some(if next.is_some() { "pointer" } else { "text" }));
    }

    fn leave(&self) {
        update_link_hover(&self.state, None);
        if let Some(list) = self.list.upgrade() {
            list.set_cursor_from_name(Some("text"));
        }
    }
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
    let context = crate::ui::launch_context::for_display(&parent.display());
    if let Err(error) = gio::AppInfo::launch_default_for_uri(uri, Some(&context)) {
        super::super::modal::show_error_dialog(
            parent,
            &crate::i18n::tr("Unable to open link"),
            &error.to_string(),
        );
    }
}
