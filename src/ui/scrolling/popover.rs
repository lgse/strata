// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};

pub(in crate::ui) fn dismiss_on_outside_scroll(popover: &gtk::Popover) {
    popover.add_controller(wheel_controller(popover));
    let root_wheel = Rc::new(RefCell::new(None::<gtk::EventControllerScroll>));
    let wheel_for_map = root_wheel.clone();
    // Outside wheel events can target the parent surface rather than the popup,
    // depending on the display backend and GTK version.
    popover.connect_map(move |popover| {
        if let Some(root) = popover.root() {
            let wheel = wheel_controller(popover);
            root.upcast::<gtk::Widget>().add_controller(wheel.clone());
            wheel_for_map.replace(Some(wheel));
        }
    });
    popover.connect_unmap(move |_| {
        if let Some(wheel) = root_wheel.borrow_mut().take()
            && let Some(widget) = wheel.widget()
        {
            widget.remove_controller(&wheel);
        }
    });
}

fn wheel_controller(popover: &gtk::Popover) -> gtk::EventControllerScroll {
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_popover = popover.downgrade();
    wheel.connect_scroll(move |controller, dx, dy| {
        let Some(popover) = weak_popover.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if !popover.is_visible() || pointer_over_widget(&popover) {
            return glib::Propagation::Proceed;
        }
        let scroll = popover.root().and_then(|root| {
            let root = root.upcast::<gtk::Widget>();
            let point = pointer_position(&root)?;
            let picked = root.pick(point.x() as f64, point.y() as f64, gtk::PickFlags::DEFAULT)?;
            listing_ancestor(&picked)
        });
        popover.popdown();
        if let Some(scroll) = scroll {
            apply_scrolled_window_wheel(&scroll, controller, dx, dy);
        }
        glib::Propagation::Stop
    });
    wheel
}

fn listing_ancestor(widget: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class("browser-listing-scroll") {
            return widget.downcast().ok();
        }
        current = widget.parent();
    }
    None
}

fn pointer_over_widget(widget: &impl IsA<gtk::Widget>) -> bool {
    let widget = widget.as_ref();
    pointer_position(widget).is_some_and(|point| {
        point.x() >= 0.0
            && point.y() >= 0.0
            && point.x() < widget.width() as f32
            && point.y() < widget.height() as f32
    })
}

fn pointer_position(widget: &gtk::Widget) -> Option<gtk::graphene::Point> {
    let native = widget.native()?;
    let surface = native.surface()?;
    let pointer = widget.display().default_seat()?.pointer()?;
    let (x, y, _) = surface.device_position(&pointer)?;
    let (ox, oy) = native.surface_transform();
    native.upcast_ref::<gtk::Widget>().compute_point(
        widget,
        &gtk::graphene::Point::new((x - ox) as f32, (y - oy) as f32),
    )
}

fn apply_scrolled_window_wheel(
    scroll: &gtk::ScrolledWindow,
    controller: &gtk::EventControllerScroll,
    mut dx: f64,
    mut dy: f64,
) {
    if controller
        .current_event_state()
        .contains(gtk::gdk::ModifierType::SHIFT_MASK)
    {
        std::mem::swap(&mut dx, &mut dy);
    }
    let unit = controller.unit();
    apply_adjustment_scroll(&scroll.hadjustment(), dx, unit);
    apply_adjustment_scroll(&scroll.vadjustment(), dy, unit);
}

fn apply_adjustment_scroll(adjustment: &gtk::Adjustment, delta: f64, unit: gtk::gdk::ScrollUnit) {
    if delta == 0.0 {
        return;
    }
    let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    adjustment.set_value(
        (adjustment.value() + scroll_delta_for_unit(delta, adjustment.page_size(), unit))
            .clamp(adjustment.lower(), max),
    );
}

fn scroll_delta_for_unit(delta: f64, page_size: f64, unit: gtk::gdk::ScrollUnit) -> f64 {
    delta
        * match unit {
            gtk::gdk::ScrollUnit::Wheel => page_size.powf(2.0 / 3.0),
            gtk::gdk::ScrollUnit::Surface => 2.5,
            _ => 1.0,
        }
}

#[cfg(test)]
mod tests;
