// SPDX-License-Identifier: MIT

use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};

use super::super::PreviewState;
use super::geometry::separator;

impl PreviewState {
    pub(in crate::ui::preview) fn resize_preview(
        self: &Rc<Self>,
        split: &gtk::Paned,
        position: i32,
    ) -> Option<i32> {
        let geometry = self.geometry(split);
        if !geometry.can_show_preview() {
            return None;
        }
        let lent = if self.slot_is_empty() {
            geometry.trailing
        } else {
            0
        };
        let width = (geometry.available - geometry.separator - position + lent)
            .clamp(geometry.minimum_width(true), geometry.maximum_width());
        self.sizing.manual_width.set(Some(width));
        self.sync_split(split);
        Some(width)
    }

    /// Every divider drag sets the session minimum. When it asks for less than the
    /// space the preview fills, the panel stays put, so the outline shows that minimum.
    fn show_minimum_outline(&self, split: &gtk::Paned, width: i32) {
        let overlay = crate::ui::modal::window_overlay(split);
        let bounds = overlay
            .as_ref()
            .and_then(|overlay| split.compute_bounds(overlay));
        let (Some(overlay), Some(bounds)) = (overlay, bounds) else {
            return;
        };
        if !self.revealer.reveals_child() {
            self.remove_minimum_outline();
            return;
        }
        let outline = self
            .sizing
            .minimum_outline
            .borrow_mut()
            .get_or_insert_with(|| minimum_outline(&overlay))
            .clone();
        outline.set_margin_start((bounds.x() + bounds.width()).round() as i32 - width);
        outline.set_margin_top(bounds.y().round() as i32);
        outline.set_size_request(width, bounds.height().round() as i32);
    }

    fn fade_minimum_outline(&self) {
        if let Some(outline) = self.sizing.minimum_outline.take() {
            super::super::super::resize_feedback::fade_out(&outline);
        }
    }

    /// The divider belongs to the preview while it is shown or Columns reserves it.
    fn lays_out_split(&self) -> bool {
        self.is_enabled() || self.reserves_column_space()
    }

    pub(super) fn can_drag_divider(&self) -> bool {
        self.lays_out_split() && !self.sizing.is_suspended()
    }

    pub(super) fn begin_divider_drag(&self, split: &gtk::Paned) {
        self.animation_generation
            .set(self.animation_generation.get().saturating_add(1));
        self.animating.set(false);
        self.sizing.resizing.set(true);
        self.sizing.dragging.set(true);
        // The outline names the minimum from here on.
        self.sizing.grip_hint.hide();
        let minimum = self.geometry(split).minimum_width(true);
        self.pane.set_width_request(minimum);
        self.slot.set_width_request(minimum);
    }

    pub(super) fn end_divider_drag(&self) {
        self.sizing.resizing.set(false);
        if self.sizing.dragging.replace(false) {
            self.fade_minimum_outline();
        }
    }

    fn remove_minimum_outline(&self) {
        if let Some(outline) = self.sizing.minimum_outline.take()
            && let Some(overlay) = outline.parent().and_downcast::<gtk::Overlay>()
        {
            overlay.remove_overlay(&outline);
        }
    }

    /// GtkPaned moved its divider; a resize in progress becomes the manual width.
    fn divider_moved(self: &Rc<Self>, split: &gtk::Paned) {
        if !self.lays_out_split() {
            return;
        }
        if self.sizing.restored_position.take() == Some(split.position()) {
            return;
        }
        if !self.sizing.resizing.replace(false) {
            return;
        }
        let dragged = split.position();
        let requested = self.resize_preview(split, dragged);
        if split.position() != dragged {
            // Filling the free space put the divider back; that echo is not a resize.
            self.sizing.restored_position.set(Some(split.position()));
        }
        if self.sizing.dragging.get()
            && let Some(width) = requested
        {
            self.show_minimum_outline(split, width);
        }
        self.sizing.resizing.set(true);
    }

    fn divider_pressed(&self, split: &gtk::Paned, event: &gtk::gdk::Event) {
        if is_primary_press(event) && self.can_drag_divider() && on_separator(split, event) {
            self.begin_divider_drag(split);
        }
    }
}

fn minimum_outline(overlay: &gtk::Overlay) -> gtk::Box {
    let outline = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outline.add_css_class("preview-minimum-outline");
    outline.set_can_target(false);
    outline.set_halign(gtk::Align::Start);
    outline.set_valign(gtk::Align::Start);
    let label = super::super::super::resize_feedback::caption(&crate::i18n::tr(
        "Preview panel minimum width",
    ));
    label.set_halign(gtk::Align::Center);
    outline.append(&label);
    overlay.add_overlay(&outline);
    outline
}

fn is_primary_press(event: &gtk::gdk::Event) -> bool {
    event.event_type() == gtk::gdk::EventType::TouchBegin
        || event
            .downcast_ref::<gtk::gdk::ButtonEvent>()
            .is_some_and(|e| e.button() == 1)
}

pub(super) fn install_resize(split: &gtk::Paned, state: &Rc<PreviewState>) {
    // Observe input without competing with GtkPaned's own drag gesture.
    let pointer = gtk::EventControllerLegacy::new();
    pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    pointer.connect_event(move |controller, event| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let Some(split) = controller.widget().and_downcast::<gtk::Paned>() else {
            return glib::Propagation::Proceed;
        };
        match event.event_type() {
            gtk::gdk::EventType::ButtonPress | gtk::gdk::EventType::TouchBegin => {
                state.divider_pressed(&split, event);
            }
            gtk::gdk::EventType::ButtonRelease
            | gtk::gdk::EventType::TouchEnd
            | gtk::gdk::EventType::TouchCancel
            | gtk::gdk::EventType::GrabBroken => state.end_divider_drag(),
            _ => {}
        }
        glib::Propagation::Proceed
    });
    split.add_controller(pointer);
    let weak = Rc::downgrade(state);
    split.connect_position_notify(move |split| {
        if let Some(state) = weak.upgrade() {
            state.divider_moved(split);
        }
    });

    // Unhandled browser keys also reach GtkPaned; only handle-focused actions are resizes.
    let weak = Rc::downgrade(state);
    split.connect_move_handle(move |split, _| {
        if split.has_focus() {
            keyboard_move_started(split, &weak);
            remember_keyboard_width(weak.clone());
        }
        false
    });
    let weak = Rc::downgrade(state);
    split.connect_cancel_position(move |split| {
        if split.has_focus() {
            remember_keyboard_width(weak.clone());
        }
        false
    });
}

fn keyboard_move_started(split: &gtk::Paned, state: &Weak<PreviewState>) {
    let Some(state) = state.upgrade().filter(|state| state.can_drag_divider()) else {
        return;
    };
    let minimum = state.geometry(split).minimum_width(true);
    state.pane.set_width_request(minimum);
    state.slot.set_width_request(minimum);
}

fn on_separator(split: &gtk::Paned, event: &gtk::gdk::Event) -> bool {
    let Some((x, y)) = event.position() else {
        return false;
    };
    let Some(native) = split.native() else {
        return false;
    };
    let (dx, dy) = native.surface_transform();
    let native: gtk::Widget = native.upcast();
    let Some(point) = native.compute_point(
        split,
        &gtk::graphene::Point::new((x + dx) as f32, (y + dy) as f32),
    ) else {
        return false;
    };
    split.pick(
        f64::from(point.x()),
        f64::from(point.y()),
        gtk::PickFlags::DEFAULT,
    ) == separator(split)
}

fn remember_keyboard_width(weak: Weak<PreviewState>) {
    let Some(state) = weak.upgrade().filter(|state| state.can_drag_divider()) else {
        return;
    };
    let Some(split) = state.split.borrow().clone() else {
        return;
    };
    let before = split.position();
    state.sizing.resizing.set(true);
    // Wait for GTK's default action handler before resuming automatic layout.
    glib::idle_add_local_once(move || {
        if let Some(state) = weak.upgrade() {
            state.sizing.resizing.set(false);
            if state.lays_out_split() && split.position() != before {
                state.resize_preview(&split, split.position());
            }
        }
    });
}
