// SPDX-License-Identifier: MIT

//! Scrolls a pointer target out from under the scroller that clips it, with the wheel a
//! user would turn: Shift for a horizontal strip such as Columns or the breadcrumbs.

use gtk::prelude::*;

use super::{
    driver::Input,
    protocol::Modifier,
    targets::{self, Located, LookupError},
};

/// GTK scrolls a discrete wheel click by the page size to the power of 2/3.
const WHEEL_STEP_EXPONENT: f64 = 2.0 / 3.0;
const MAX_CLICKS: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Axis {
    Horizontal,
    Vertical,
}

/// How far the content must scroll to show `point`: the whole span when it fits, else the
/// point centred. Positive scrolls forward (right or down); `None` when it already shows.
pub(super) fn travel(span: (f64, f64), point: f64, viewport: (f64, f64)) -> Option<f64> {
    let (start, length) = span;
    let (view_start, view_length) = viewport;
    let view_end = view_start + view_length;
    if point >= view_start && point < view_end {
        return None;
    }
    Some(if length > view_length {
        point - (view_start + view_length / 2.0)
    } else if point >= view_end {
        start + length - view_end
    } else {
        start - view_start
    })
}

pub(super) fn wheel_clicks(distance: f64, page_size: f64) -> u32 {
    let step = page_size.max(1.0).powf(WHEEL_STEP_EXPONENT);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=12 just before"
    )]
    let clicks = (distance.abs() / step).ceil().clamp(1.0, MAX_CLICKS) as u32;
    clicks
}

struct Clipped {
    scroller: gtk::ScrolledWindow,
    viewport: targets::Rect,
    axis: Axis,
    distance: f64,
}

impl Clipped {
    fn adjustment(&self) -> gtk::Adjustment {
        match self.axis {
            Axis::Horizontal => self.scroller.hadjustment(),
            Axis::Vertical => self.scroller.vadjustment(),
        }
    }
}

fn scrollable(adjustment: &gtk::Adjustment) -> bool {
    adjustment.upper() - adjustment.page_size() > adjustment.lower()
}

/// The innermost scroller that leaves `point` outside its viewport along an axis it can
/// scroll.
fn clipping(
    located: &Located,
    (x, y): (f64, f64),
    input: &Input<'_>,
) -> Result<Option<Clipped>, String> {
    let rect = located.rect;
    for ancestor in std::iter::successors(located.widget.parent(), WidgetExt::parent) {
        let Ok(scroller) = ancestor.downcast::<gtk::ScrolledWindow>() else {
            continue;
        };
        let viewport = match targets::screen_rect(scroller.upcast_ref(), input.injector) {
            Ok(viewport) => viewport,
            Err(LookupError::Missing(_)) => continue,
            Err(LookupError::Broken(error)) => return Err(error),
        };
        let axes = [
            (
                Axis::Horizontal,
                scroller.hadjustment(),
                (rect.x, rect.width),
                x,
                (viewport.x, viewport.width),
            ),
            (
                Axis::Vertical,
                scroller.vadjustment(),
                (rect.y, rect.height),
                y,
                (viewport.y, viewport.height),
            ),
        ];
        for (axis, adjustment, span, point, view) in axes {
            if let Some(distance) = travel(span, point, view)
                && scrollable(&adjustment)
            {
                return Ok(Some(Clipped {
                    scroller,
                    viewport,
                    axis,
                    distance,
                }));
            }
        }
    }
    Ok(None)
}

impl Input<'_> {
    /// Turns the wheel over the scroller clipping `point` on `located`; false when nothing
    /// clips it or the scroller would not move, so the caller aims where the target is.
    pub(super) async fn scroll_onto_screen(
        &self,
        located: &Located,
        point: (f64, f64),
    ) -> Result<bool, String> {
        let Some(clipped) = clipping(located, point, self)? else {
            return Ok(false);
        };
        let window = targets::screen_rect(self.window.upcast_ref(), self.injector)
            .map_err(|error| error.message().to_owned())?;
        let left = clipped.viewport.x.max(window.x);
        let top = clipped.viewport.y.max(window.y);
        let right = (clipped.viewport.x + clipped.viewport.width).min(window.x + window.width);
        let bottom = (clipped.viewport.y + clipped.viewport.height).min(window.y + window.height);
        if right <= left || bottom <= top {
            return Ok(false);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "screen coordinates are far inside i64"
        )]
        let at = (
            ((left + right) / 2.0).floor() as i64,
            ((top + bottom) / 2.0).floor() as i64,
        );
        let adjustment = clipped.adjustment();
        let before = adjustment.value();
        let modifiers: &[Modifier] = match clipped.axis {
            Axis::Horizontal => &[Modifier::Shift],
            Axis::Vertical => &[],
        };
        self.wheel_at(
            at,
            wheel_clicks(clipped.distance, adjustment.page_size()),
            clipped.distance > 0.0,
            modifiers,
        )
        .await?;
        super::settle(Some(self.window)).await;
        Ok((adjustment.value() - before).abs() > f64::EPSILON)
    }
}
