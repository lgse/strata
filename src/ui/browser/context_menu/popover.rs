// SPDX-License-Identifier: MIT

use super::{ResolvedContextTrigger, keyboard};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

const CONTEXT_MENU_EDGE_MARGIN: i32 = 8;

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

fn natural_size(widget: &impl IsA<gtk::Widget>, orientation: gtk::Orientation) -> i32 {
    widget.measure(orientation, -1).1
}

/// A hidden popover measures as empty, and its scrolled window can under-report its
/// child, so the menu frame and the scrolled content are measured directly.
#[derive(Clone, Copy, PartialEq, Eq)]
struct MenuSize {
    content: i32,
    chrome: i32,
    width: i32,
}

impl MenuSize {
    fn of(scroll: &gtk::ScrolledWindow) -> Self {
        let frame = scroll.parent().unwrap_or_else(|| scroll.clone().upcast());
        Self {
            content: scroll
                .child()
                .map_or(0, |child| natural_size(&child, gtk::Orientation::Vertical)),
            chrome: natural_size(&frame, gtk::Orientation::Vertical)
                - natural_size(scroll, gtk::Orientation::Vertical),
            width: natural_size(&frame, gtk::Orientation::Horizontal),
        }
    }
}

struct Placement {
    position: gtk::PositionType,
    offset: (i32, i32),
    max_content_height: i32,
}

impl Placement {
    fn apply(&self, popover: &gtk::Popover, scroll: &gtk::ScrolledWindow) {
        popover.set_position(self.position);
        popover.set_offset(self.offset.0, self.offset.1);
        scroll.set_max_content_height(self.max_content_height);
    }
}

/// Keeps a menu opened at `point` inside `window`: below the point when it fits there,
/// above when only that side fits, else slid up to show it whole. It scrolls only
/// when the window is shorter than the menu. GTK centres it on the point horizontally.
fn fit_in_window(window: &gtk::Widget, point: &gtk::graphene::Point, size: MenuSize) -> Placement {
    let (height, width) = (size.content + size.chrome, size.width);
    let (x, y) = (point.x().round() as i32, point.y().round() as i32);
    let bottom = window.height() - CONTEXT_MENU_EDGE_MARGIN;
    let right = window.width() - CONTEXT_MENU_EDGE_MARGIN;
    let below = bottom - y;
    let above = y - CONTEXT_MENU_EDGE_MARGIN;
    let (position, shift_y, room) = if height <= below {
        (gtk::PositionType::Bottom, 0, below)
    } else if height <= above {
        (gtk::PositionType::Top, 0, above)
    } else {
        let top = (bottom - height).max(CONTEXT_MENU_EDGE_MARGIN);
        (gtk::PositionType::Bottom, top - y, bottom - top)
    };
    let left = x - width / 2;
    let shift_x = left.clamp(
        CONTEXT_MENU_EDGE_MARGIN,
        (right - width).max(CONTEXT_MENU_EDGE_MARGIN),
    ) - left;
    Placement {
        position,
        offset: (shift_x, shift_y),
        max_content_height: (room - size.chrome).max(1),
    }
}

/// Fits the menu before it opens, then again on each frame it grows: some items only
/// take their size once the menu is shown. Stops once its size holds.
fn keep_in_window(
    popover: &gtk::Popover,
    scroll: &gtk::ScrolledWindow,
    window: &gtk::Widget,
    point: gtk::graphene::Point,
) {
    let size = MenuSize::of(scroll);
    fit_in_window(window, &point, size).apply(popover, scroll);
    let fitted = Cell::new(size);
    let pointing = popover.pointing_to();
    let (scroll, window) = (scroll.downgrade(), window.downgrade());
    popover.add_tick_callback(move |popover, _| {
        let (Some(scroll), Some(window)) = (scroll.upgrade(), window.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        let size = MenuSize::of(&scroll);
        if popover.pointing_to() != pointing || size == fitted.replace(size) {
            return glib::ControlFlow::Break;
        }
        fit_in_window(&window, &point, size).apply(popover, &scroll);
        glib::ControlFlow::Continue
    });
}

fn point_at(point: &gtk::graphene::Point) -> gtk::gdk::Rectangle {
    gtk::gdk::Rectangle::new(point.x().round() as i32, point.y().round() as i32, 1, 1)
}

fn position_model_context_popover(popover: &gtk::Popover, anchor: &gtk::Widget, x: f64, y: f64) {
    let Some(parent) = popover.parent() else {
        return;
    };
    let click = gtk::graphene::Point::new(x as f32, y as f32);
    let Some(point) = anchor.compute_point(&parent, &click) else {
        return;
    };
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_offset(0, 0);
    popover.set_pointing_to(Some(&point_at(&point)));
    if let Some(scroll) = popover.child().and_downcast::<gtk::ScrolledWindow>() {
        scroll.set_propagate_natural_height(true);
        if let Some(window) = crate::ui::modal::window_overlay(anchor)
            && let Some(in_window) = anchor.compute_point(&window, &click)
        {
            keep_in_window(popover, &scroll, window.upcast_ref(), in_window);
        }
    }
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
    popover.set_pointing_to(Some(&point_at(&point)));
    keep_in_window(popover, scroll, overlay.upcast_ref(), point);
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
