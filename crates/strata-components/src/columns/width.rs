// SPDX-License-Identifier: MIT

use std::time::Duration;

use strata_kit::look::Look;

/// One standard column, in base points: the narrowest a column draws, and the width new columns open at with nothing saved.
pub const STANDARD: f32 = 300.0;

/// How far a column's resize edge reaches either side of the line between columns.
pub const EDGE_REACH: f32 = 6.0;

/// How long the pointer rests on an edge before the "Column width" caption shows.
pub const CAPTION_DELAY: Duration = Duration::from_millis(400);

/// Two presses on one column edge within this many seconds of the host's clock fit the column to its content.
pub const AUTOFIT_WITHIN: f64 = 0.4;

/// A resize of column `depth`'s edge in progress, in base points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Resize {
    pub depth: usize,
    /// The width at the press, put back if the drag is cancelled.
    pub start: f32,
    /// The width now.
    pub live: f32,
}

/// The factor widths are drawn at: the text size over the base.
pub fn scale(look: &Look) -> f32 {
    crate::list_table::text_scale(look)
}

/// A width from a saved or hand-edited value, in base points.
pub fn sane(base: f32) -> Option<f32> {
    (base.is_finite() && base > 0.0).then(|| base.max(STANDARD))
}

/// The width `base` draws at, at `scale`: never below one standard column.
pub fn drawn(base: f32, scale: f32) -> f32 {
    base.max(STANDARD) * scale
}

/// The width a drag of `travel` points from `start` (base points) gives, at `scale`, under the area's `cap` (points).
pub fn dragged(start: f32, travel: f32, scale: f32, cap: Option<f32>) -> f32 {
    let w = start + travel / scale;
    let w = match cap {
        Some(cap) => w.min((cap / scale).max(STANDARD)),
        None => w,
    };
    w.max(STANDARD)
}

/// The width, in base points, that content `drawn` points wide fits in at `scale`, under the area's `cap` (points).
pub fn fitted(drawn: f32, scale: f32, cap: Option<f32>) -> f32 {
    dragged(0.0, drawn.ceil(), scale, cap)
}
