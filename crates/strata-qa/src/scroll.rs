// SPDX-License-Identifier: MIT

//! How far a scroller must turn to show a point, in wheel clicks.

/// GTK scrolls a discrete wheel click by the page size to the power of 2/3.
const WHEEL_STEP_EXPONENT: f64 = 2.0 / 3.0;
const MAX_CLICKS: f64 = 12.0;

/// How far the content must scroll to show `point`: the whole span when it fits, else the
/// point centred. Positive scrolls forward (right or down); `None` when it already shows.
pub fn travel(span: (f64, f64), point: f64, viewport: (f64, f64)) -> Option<f64> {
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

pub fn wheel_clicks(distance: f64, page_size: f64) -> u32 {
    let step = page_size.max(1.0).powf(WHEEL_STEP_EXPONENT);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=12 just before"
    )]
    let clicks = (distance.abs() / step).ceil().clamp(1.0, MAX_CLICKS) as u32;
    clicks
}
