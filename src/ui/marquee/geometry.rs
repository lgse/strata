// SPDX-License-Identifier: MIT

use gtk::graphene;
use gtk::prelude::*;

/// Distance from a viewport edge at which a marquee drag starts scrolling.
const AUTO_SCROLL_MARGIN: f64 = 28.0;
/// Largest scroll step, in pixels, applied per auto-scroll frame.
const AUTO_SCROLL_MAX_STEP: f64 = 24.0;

/// The marquee rectangle in scroll-content coordinates.
#[derive(Clone, Copy)]
pub(super) struct Band {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Band {
    pub(super) fn between(anchor: (f64, f64), current: (f64, f64)) -> Self {
        Self {
            left: anchor.0.min(current.0),
            right: anchor.0.max(current.0),
            top: anchor.1.min(current.1),
            bottom: anchor.1.max(current.1),
        }
    }

    /// A band with zero area still resolves against the item under it, so a press
    /// that has not moved yet behaves like a click.
    pub(super) fn intersects(&self, bounds: &graphene::Rect) -> bool {
        f64::from(bounds.x()) < self.right
            && f64::from(bounds.x() + bounds.width()) > self.left
            && f64::from(bounds.y()) < self.bottom
            && f64::from(bounds.y() + bounds.height()) > self.top
    }
}

/// A band rectangle positioned inside the overlay it is drawn in.
pub(super) struct BandBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub(super) fn bounds_contain(bounds: &graphene::Rect, (x, y): (f64, f64)) -> bool {
    x >= f64::from(bounds.x())
        && x < f64::from(bounds.x() + bounds.width())
        && y >= f64::from(bounds.y())
        && y < f64::from(bounds.y() + bounds.height())
}

pub(super) fn content_point(scroll: &gtk::ScrolledWindow, point: (f64, f64)) -> (f64, f64) {
    (
        point.0 + scroll.hadjustment().value(),
        point.1 + scroll.vadjustment().value(),
    )
}

pub(super) fn advance(adjustment: &gtk::Adjustment, step: f64) -> bool {
    if step == 0.0 {
        return false;
    }
    let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    let target = (adjustment.value() + step).clamp(adjustment.lower(), upper);
    if (target - adjustment.value()).abs() < f64::EPSILON {
        return false;
    }
    adjustment.set_value(target);
    true
}

pub(super) fn translate(
    from: &impl IsA<gtk::Widget>,
    to: &impl IsA<gtk::Widget>,
    point: (f64, f64),
) -> Option<(f64, f64)> {
    let point = graphene::Point::new(point.0 as f32, point.1 as f32);
    from.as_ref()
        .compute_point(to, &point)
        .map(|point| (f64::from(point.x()), f64::from(point.y())))
}

/// Scroll step for a pointer coordinate inside a viewport `size` pixels long.
/// Negative values scroll towards the start, positive towards the end.
pub(super) fn auto_scroll_step(position: f64, size: f64) -> f64 {
    if size <= AUTO_SCROLL_MARGIN * 2.0 {
        return 0.0;
    }
    let overshoot = if position < AUTO_SCROLL_MARGIN {
        position - AUTO_SCROLL_MARGIN
    } else if position > size - AUTO_SCROLL_MARGIN {
        position - (size - AUTO_SCROLL_MARGIN)
    } else {
        return 0.0;
    };
    (overshoot / AUTO_SCROLL_MARGIN).clamp(-1.0, 1.0) * AUTO_SCROLL_MAX_STEP
}

/// Clips a band rectangle to the overlay it is drawn in, returning `None` when
/// none of it remains visible.
pub(super) fn band_placement(
    band: BandBox,
    (overlay_width, overlay_height): (f64, f64),
) -> Option<(i32, i32, i32, i32)> {
    let left = band.x.max(0.0);
    let top = band.y.max(0.0);
    let right = (band.x + band.width).min(overlay_width);
    let bottom = (band.y + band.height).min(overlay_height);
    if right < left || bottom < top {
        return None;
    }
    Some((
        left.round() as i32,
        top.round() as i32,
        (right - left).round().max(1.0) as i32,
        (bottom - top).round().max(1.0) as i32,
    ))
}
