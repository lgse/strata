// SPDX-License-Identifier: MIT

use super::{ResolvedContextTrigger, keyboard};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

const CONTEXT_MENU_EDGE_MARGIN: i32 = 16;

pub(in crate::ui) fn context_menu_popover(
    content: &impl IsA<gtk::Widget>,
) -> (gtk::Popover, gtk::ScrolledWindow) {
    let viewport = gtk::Viewport::builder()
        .child(content)
        .scroll_to_focus(true)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&viewport)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .build();
    scroll.add_css_class("context-menu-scroll");
    let popover = gtk::Popover::builder()
        .child(&scroll)
        .autohide(true)
        .has_arrow(false)
        .build();
    keyboard::install(&popover);
    (popover, scroll)
}

pub(in crate::ui) fn show_context_popover(
    popover: &gtk::Popover,
    scroll: &gtk::ScrolledWindow,
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
) {
    present_context_popover(popover, scroll, anchor, (x, y));
}

pub(super) fn show_model_context_popover(
    popover: &gtk::Popover,
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
) {
    if popover.parent().is_none() {
        popover.set_parent(anchor);
    }
    position_model_context_popover(popover, anchor, x, y);
    popover.popup();
}

// Capped by the monitor, not the window: the popup flips above the click or
// shrinks to fit the screen.
fn monitor_menu_height(widget: &impl IsA<gtk::Widget>) -> i32 {
    let monitor = widget
        .native()
        .and_then(|native| native.surface())
        .and_then(|surface| surface.display().monitor_at_surface(&surface));
    monitor
        .map_or(i32::MAX / 2, |monitor| monitor.geometry().height())
        .saturating_sub(CONTEXT_MENU_EDGE_MARGIN * 2)
        .max(1)
}

fn point_at(point: &gtk::graphene::Point) -> gtk::gdk::Rectangle {
    gtk::gdk::Rectangle::new(point.x().round() as i32, point.y().round() as i32, 1, 1)
}

fn position_model_context_popover(popover: &gtk::Popover, anchor: &gtk::Widget, x: f64, y: f64) {
    let Some(parent) = popover.parent() else {
        return;
    };
    let Some(point) = anchor.compute_point(&parent, &gtk::graphene::Point::new(x as f32, y as f32))
    else {
        return;
    };
    popover.set_position(gtk::PositionType::Bottom);
    if let Some(scroll) = popover.child().and_downcast::<gtk::ScrolledWindow>() {
        scroll.set_max_content_height(monitor_menu_height(anchor));
        scroll.set_propagate_natural_height(true);
    }
    popover.set_pointing_to(Some(&point_at(&point)));
}

fn present_context_popover(
    popover: &gtk::Popover,
    scroll: &gtk::ScrolledWindow,
    anchor: &gtk::Widget,
    (x, y): (f64, f64),
) {
    let Some(overlay) = crate::ui::modal::window_overlay(anchor) else {
        return;
    };
    if popover.parent().is_none() {
        overlay.add_overlay(popover);
    }
    let point = anchor
        .compute_point(&overlay, &gtk::graphene::Point::new(x as f32, y as f32))
        .unwrap_or(gtk::graphene::Point::new(x as f32, y as f32));
    popover.set_position(gtk::PositionType::Bottom);
    scroll.set_max_content_height(monitor_menu_height(anchor));
    popover.set_pointing_to(Some(&point_at(&point)));
    popover.popup();
    popover.present();
}

type WeakContextTrigger = Weak<dyn Fn(f64, f64, Option<&gtk::GestureClick>) -> bool>;

type InstalledRetarget =
    Rc<RefCell<Option<(glib::WeakRef<gtk::Widget>, gtk::EventControllerLegacy)>>>;

/// The widget-relative point of a secondary-button release, when it lands while
/// the popover is hidden.
fn hidden_secondary_release(
    event: &gtk::gdk::Event,
    popover: &gtk::Popover,
    (widget, root): (&gtk::Widget, &gtk::Widget),
) -> Option<(f64, f64)> {
    let button = event
        .downcast_ref::<gtk::gdk::ButtonEvent>()
        .filter(|button| {
            event.event_type() == gtk::gdk::EventType::ButtonRelease && button.button() == 3
        })?;
    if popover.is_visible() {
        return None;
    }
    let (x, y) = button.position()?;
    root.compute_point(widget, &gtk::graphene::Point::new(x as f32, y as f32))
        .map(|point| (f64::from(point.x()), f64::from(point.y())))
}

fn retarget_controller(
    popover: &gtk::Popover,
    widget: &gtk::Widget,
    root: &gtk::Widget,
    reopen: WeakContextTrigger,
) -> gtk::EventControllerLegacy {
    let input = gtk::EventControllerLegacy::new();
    input.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_popover = popover.downgrade();
    let weak_widget = widget.downgrade();
    let weak_root = root.downgrade();
    input.connect_event(move |_, event| {
        let (Some(popover), Some(widget), Some(root)) = (
            weak_popover.upgrade(),
            weak_widget.upgrade(),
            weak_root.upgrade(),
        ) else {
            return glib::Propagation::Proceed;
        };
        if let Some((x, y)) = hidden_secondary_release(event, &popover, (&widget, &root))
            && let Some(reopen) = reopen.upgrade()
        {
            glib::idle_add_local_once(move || {
                reopen(x, y, None);
            });
        }
        glib::Propagation::Proceed
    });
    input
}

/// GTK consumes the outside press while autohiding a popover, but the release
/// reaches the window. Re-resolve that release so one secondary click retargets.
pub(super) fn install_secondary_release_retarget(
    popover: &gtk::Popover,
    widget: &gtk::Widget,
    reopen: ResolvedContextTrigger,
) {
    let installed: InstalledRetarget = Rc::new(RefCell::new(None));
    let installed_for_show = installed.clone();
    // The trigger owns the menu; its show handler and root controller must not retain it.
    let reopen = Rc::downgrade(&reopen);
    let weak_widget = widget.downgrade();
    let weak_popover = popover.downgrade();
    popover.connect_show(move |_| {
        if installed_for_show.borrow().is_some() {
            return;
        }
        let (Some(widget), Some(popover)) = (weak_widget.upgrade(), weak_popover.upgrade()) else {
            return;
        };
        let Some(root) = widget
            .root()
            .and_then(|root| root.dynamic_cast::<gtk::Widget>().ok())
        else {
            return;
        };
        let input = retarget_controller(&popover, &widget, &root, reopen.clone());
        root.add_controller(input.clone());
        installed_for_show.replace(Some((root.downgrade(), input)));
    });
    widget.connect_unmap(move |_| {
        if let Some((root, input)) = installed.borrow_mut().take()
            && let Some(root) = root.upgrade()
        {
            root.remove_controller(&input);
        }
    });
}
