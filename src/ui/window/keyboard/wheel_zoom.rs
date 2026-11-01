// SPDX-License-Identifier: MIT

//! Ctrl+wheel zoom for interface text and Icons thumbnails.

/// Accumulates fractional Ctrl+wheel deltas into whole steps so smooth devices
/// zoom in the same increments as wheel clicks: interface text, or Icons
/// thumbnails with Shift. Resets when the input reverses direction, changes
/// units or target, or is not claimed for zooming.
use std::cell::Cell;

use gtk::gdk::ModifierType as Modifiers;
use gtk::{glib::Propagation, prelude::*};

use crate::ui::{browser::BrowserView, browser_modes::ThumbnailResize};

#[derive(Default)]
pub(in crate::ui) struct WheelZoom {
    pub(super) pending: Cell<f64>,
    pub(super) direction: Cell<i8>,
    pub(super) unit: Cell<Option<gtk::gdk::ScrollUnit>>,
    pub(super) thumbnails: Cell<bool>,
}

impl WheelZoom {
    /// Surface-unit deltas arrive in pixels; 10 px make one wheel step, the
    /// same mapping GTK's discrete conversion uses.
    const SURFACE_PIXELS_PER_STEP: f64 = 10.0;

    pub(super) fn reset(&self) {
        self.pending.set(0.0);
        self.direction.set(0);
    }

    pub(super) fn accumulate(&self, unit: gtk::gdk::ScrollUnit, dy: f64) -> i32 {
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

    pub(in crate::ui) fn handle(
        &self,
        preferences: &crate::ui::preferences::PreferenceManager,
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
        if !modifiers.contains(Modifiers::CONTROL_MASK)
            || modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            || target.as_ref().is_some_and(inside_pdf_scroll)
            || (thumbnails && view.view_mode() != crate::ui::browser_modes::BrowserMode::Icons)
        {
            self.reset();
            return Propagation::Proceed;
        }
        if self.thumbnails.replace(thumbnails) != thumbnails {
            self.reset();
        }
        let steps = self.accumulate(unit, delta);
        if steps != 0 {
            if thumbnails {
                view.resize_icons_thumbnails(ThumbnailResize::Step(-steps));
            } else {
                view.set_text_size(preferences.text_size().stepped(-steps));
            }
        }
        Propagation::Stop
    }
}

pub(super) fn inside_pdf_scroll(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class("preview-pdf-scroll") {
            return true;
        }
        current = widget.parent();
    }
    false
}
