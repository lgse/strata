//! The page column's zoom: a level from 1 to 4 times the fitted width, as Strata's PDF zoom
//! (`src/ui/preview/pdf/zoom.rs`, upstream/main e9a8993b7), on C14a's zoom math.
//!
//! The fitted width is the pane's width, at most [`CONTENT_MAX_WIDTH`], less a gap each side.
//! Ctrl+wheel multiplies the level by `e^0.14` a notch (40 points of Rune's wheel; a wheel
//! turned away zooms in) and keeps the view's centre where it was, as Strata does
//! (`preserve_pdf_view_center`).

use crate::preview_drawer::CONTENT_MAX_WIDTH;
use crate::viewers::viewport::zoom::{ZoomLimits, wheel_factor_per};

/// The gap around each page, in points: Strata's `PDF_PAGE_GAP` (6).
pub const GAP: f32 = 6.0;

/// Strata's PDF zoom range: 1× to 4× the fitted width.
pub const LIMITS: ZoomLimits = ZoomLimits { min: 1.0, max: 4.0 };

/// How long the zoom rests before pages render again at it, in seconds: a Ctrl+wheel gesture
/// asks once, at its end, not once a notch.
pub const SETTLE: f64 = 0.15;

/// Natural log of the zoom factor per point of wheel: Strata's 0.14 a notch, a notch 40 points.
const PER_POINT: f32 = 0.14 / 40.0;

/// The level after a Ctrl+wheel of `dy` points from `level`.
pub fn after_wheel(level: f32, dy: f32) -> f32 {
    LIMITS.clamp(level * wheel_factor_per(dy, PER_POINT))
}

/// The fitted page width for a pane `pane` points wide.
pub fn fitted_width(pane: f32) -> f32 {
    (pane.min(CONTENT_MAX_WIDTH) - 2.0 * GAP).max(1.0)
}

/// A page's drawn width at `level` in a pane `pane` points wide, in whole points.
pub fn drawn_width(pane: f32, level: f32) -> f32 {
    (fitted_width(pane) * level).round()
}

/// A page's drawn height for its `size` at drawn width `w`.
pub fn drawn_height(w: f32, size: [f32; 2]) -> f32 {
    (w * size[1] / size[0]).round().max(1.0)
}
