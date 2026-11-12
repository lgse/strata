// SPDX-License-Identifier: MIT

//! Ctrl+wheel zoom for interface text and Icons thumbnails.

use std::{cell::Cell, rc::Rc};

use gtk::{gdk::ModifierType as Modifiers, glib::Propagation, prelude::*};

use crate::ui::{
    browser::{BrowserView, WeakBrowserView},
    browser_modes::{BrowserMode, ThumbnailResize},
    preferences::PreferenceManager,
};

use super::accepts_input;

/// Ctrl+wheel mirrors the Ctrl +/- text-size shortcut, and Ctrl+Shift+wheel
/// the Ctrl+Alt +/- thumbnail one. Capture phase so scrolled windows cannot
/// consume it first; the PDF preview's own zoom is left alone by passing
/// through scrolls inside its scroll container. DISCRETE is avoided: it
/// swallows sub-step smooth deltas before they reach descendant scrolled
/// windows, killing touchpad scrolling.
pub(super) fn zoom_controller(
    root: &gtk::Widget,
    preferences: Rc<PreferenceManager>,
    view: WeakBrowserView,
    input: Option<super::PaneInput>,
) -> gtk::EventControllerScroll {
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let zoom = Rc::new(WheelZoom::default());
    let wheel_root = root.downgrade();
    wheel.connect_scroll(move |controller, dx, dy| {
        if !accepts_input(&wheel_root) || input.as_ref().is_some_and(|input| !input.get()) {
            return Propagation::Proceed;
        }
        let Some(view) = view.upgrade() else {
            return Propagation::Proceed;
        };
        let modifiers = controller.current_event_state();
        zoom.handle(
            &preferences,
            &view,
            modifiers,
            controller.unit(),
            pointer_target(controller, modifiers),
            (dx, dy),
        )
    });
    wheel
}

/// Scroll events carry no position, so ask the seat where the pointer is.
fn pointer_target(
    controller: &gtk::EventControllerScroll,
    modifiers: Modifiers,
) -> Option<gtk::Widget> {
    controller
        .current_event()
        .filter(|_| modifiers.contains(Modifiers::CONTROL_MASK))
        .and_then(|event| {
            let surface = event.surface()?;
            let device = event.seat()?.pointer()?;
            let (x, y, _) = surface.device_position(&device)?;
            let native = gtk::Native::for_surface(&surface)?;
            let (dx, dy) = native.surface_transform();
            native.pick(x - dx, y - dy, gtk::PickFlags::DEFAULT)
        })
}

/// Accumulates fractional Ctrl+wheel deltas into whole steps so smooth devices
/// zoom in the same increments as wheel clicks: interface text, or Icons
/// thumbnails with Shift. Resets when the input reverses direction, changes
/// units or target, or is not claimed for zooming.
#[derive(Default)]
pub(in crate::ui::window) struct WheelZoom {
    pending: Cell<f64>,
    direction: Cell<i8>,
    unit: Cell<Option<gtk::gdk::ScrollUnit>>,
    thumbnails: Cell<bool>,
}

impl WheelZoom {
    /// Surface-unit deltas arrive in pixels; 10 px make one wheel step, the
    /// same mapping GTK's discrete conversion uses.
    const SURFACE_PIXELS_PER_STEP: f64 = 10.0;

    fn reset(&self) {
        self.pending.set(0.0);
        self.direction.set(0);
    }

    fn accumulate(&self, unit: gtk::gdk::ScrollUnit, dy: f64) -> i32 {
        let direction = dy.signum() as i8;
        let unit_changed = self.unit.replace(Some(unit)) != Some(unit);
        let direction_changed = direction != 0 && self.direction.replace(direction) != direction;
        if unit_changed || direction_changed {
            self.pending.set(0.0);
        }
        let delta = if unit == gtk::gdk::ScrollUnit::Surface {
            dy / Self::SURFACE_PIXELS_PER_STEP
        } else {
            dy
        };
        let accumulated = self.pending.get() + delta;
        let steps = accumulated.trunc() as i32;
        self.pending.set(accumulated - f64::from(steps));
        steps
    }

    pub(in crate::ui::window) fn handle(
        &self,
        preferences: &PreferenceManager,
        view: &BrowserView,
        modifiers: Modifiers,
        unit: gtk::gdk::ScrollUnit,
        target: Option<gtk::Widget>,
        (dx, dy): (f64, f64),
    ) -> Propagation {
        let thumbnails = modifiers.contains(Modifiers::SHIFT_MASK);
        // Some systems turn Shift+wheel into horizontal scrolling.
        let delta = if thumbnails && dy == 0.0 { dx } else { dy };
        if delta == 0.0 {
            return Propagation::Proceed;
        }
        if !claims_zoom(view, modifiers, target.as_ref(), thumbnails) {
            self.reset();
            return Propagation::Proceed;
        }
        if self.thumbnails.replace(thumbnails) != thumbnails {
            self.reset();
        }
        let steps = self.accumulate(unit, delta);
        if steps != 0 {
            zoom_by(preferences, view, thumbnails, -steps);
        }
        Propagation::Stop
    }
}

/// Only Ctrl alone (with Shift for thumbnails, in Icons) zooms, and never over
/// the PDF preview, which zooms itself.
fn claims_zoom(
    view: &BrowserView,
    modifiers: Modifiers,
    target: Option<&gtk::Widget>,
    thumbnails: bool,
) -> bool {
    if !modifiers.contains(Modifiers::CONTROL_MASK) {
        return false;
    }
    if modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
        return false;
    }
    if target.is_some_and(inside_pdf_scroll) {
        return false;
    }
    !thumbnails || view.view_mode() == BrowserMode::Icons
}

fn zoom_by(preferences: &PreferenceManager, view: &BrowserView, thumbnails: bool, steps: i32) {
    if thumbnails {
        view.resize_icons_thumbnails(ThumbnailResize::Step(steps));
    } else {
        view.set_text_size(preferences.text_size().stepped(steps));
    }
}

fn inside_pdf_scroll(widget: &gtk::Widget) -> bool {
    std::iter::successors(Some(widget.clone()), gtk::Widget::parent)
        .any(|widget| widget.has_css_class("preview-pdf-scroll"))
}
