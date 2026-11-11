//! Middle-click autoscroll's speed curve and settings. Pure maths; speeds are in points a
//! second.

use std::time::Duration;

use rune::Spring;

/// How autoscroll starts and how fast it gets.
///
/// Fields may be added, so a host outside this crate starts from the defaults and sets fields.
/// A collection reads it by type (`Ctx::settings`), as the marquee reads its own: the host
/// passes it with `Collection::settings` each time it builds the collection.
///
/// **Hosts set [`AutoscrollSettings::margin`] from `slots::collection::row_inset(look)`**
/// when their rows keep the collection element's inset, and pass `Look::animations` to
/// [`Autoscroll::drive`](super::Autoscroll::drive).
///
/// Used by: C2, C3, C4, and hosts.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct AutoscrollSettings {
    /// How far (points) the pointer moves from the anchor before anything scrolls: Strata's
    /// `DEAD_ZONE`, 12.
    pub dead_zone: f32,
    /// How much further it moves until the speed is greatest: Strata's
    /// `FULL_SPEED_DISTANCE`, 220.
    pub span: f32,
    /// The greatest speed, points a second: Strata's `MAX_STEP` of 32 pt per 16 ms frame
    /// (`FRAME_INTERVAL`), 2,000.
    pub max_speed: f32,
    /// The spring each frame's glide runs on: Rune's `GLIDE`, as the marquee's edge scroll.
    pub glide: Spring,
    /// The frame interval assumed for a first step; later steps use the interval measured
    /// between them. 1/60 s.
    pub frame: Duration,
    /// The blank strip along each side of a row, in points, which counts as background: Strata
    /// starts autoscroll anywhere outside a row (`ui::pointer::is_background`), and its rows
    /// sit 5 pt in from the list's sides (`.file-list row`).
    pub margin: f32,
}

impl AutoscrollSettings {
    /// Strata's values (`src/ui/scrolling.rs`, `upstream/main` 35ee677ab).
    pub const STRATA: AutoscrollSettings = AutoscrollSettings {
        dead_zone: 12.0,
        span: 220.0,
        max_speed: 2000.0,
        glide: rune::tok::motion::GLIDE,
        frame: Duration::from_nanos(16_666_667),
        margin: 5.0,
    };
}

impl Default for AutoscrollSettings {
    fn default() -> Self {
        Self::STRATA
    }
}

/// The scroll speed, in points a second, for a pointer `delta` points from the anchor along
/// one axis: zero within the dead zone, then growing with the square of the distance past it
/// up to [`AutoscrollSettings::max_speed`] at `dead_zone + span`, as Strata's
/// `autoscroll_step` does (so small movements stay controllable). Its sign is the delta's.
pub fn speed(delta: f32, s: &AutoscrollSettings) -> f32 {
    let travel = delta.abs() - s.dead_zone;
    // Written so that NaN fails the comparison and lands on zero.
    if !(travel > 0.0 && s.span > 0.0) {
        return 0.0;
    }
    let ratio = (travel / s.span).clamp(0.0, 1.0);
    delta.signum() * ratio * ratio * s.max_speed
}
