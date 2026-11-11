//! The drag autoscroll's speed curve: Strata's `drag_scroll.rs` (`edge_direction`,
//! `scroll_speed`), `upstream/main` 35ee677ab. Pure maths; speeds in points a second.

use std::time::Duration;

/// How far from an edge (points) a held file drag starts to scroll.
pub const EDGE_MARGIN: f32 = 44.0;

/// Within this distance of the edge the speed is greatest.
pub const EDGE_SATURATION: f32 = 16.0;

/// The column strip's greatest speed.
pub const MAX_HORIZONTAL_SPEED: f32 = 720.0;

/// A column's listing's greatest speed.
pub const MAX_VERTICAL_SPEED: f32 = 520.0;

/// How long the speed takes to reach its greatest, held in one direction.
pub const RAMP: Duration = Duration::from_millis(350);

/// The share of the speed a scroll starts at.
pub const RAMP_FLOOR: f32 = 0.35;

/// How near an edge `position` is, along a span `size` long: -1 at the leading edge (and
/// within [`EDGE_SATURATION`] of it), falling to 0 at [`EDGE_MARGIN`] from it; +1 likewise at
/// the trailing edge; 0 between them, and in a span too short for two margins.
pub fn edge_direction(position: f32, size: f32) -> f32 {
    if size.is_nan() || size <= EDGE_MARGIN * 2.0 {
        return 0.0;
    }
    let near = |dist: f32| {
        let p = if dist <= EDGE_SATURATION {
            1.0
        } else {
            1.0 - (dist - EDGE_SATURATION) / (EDGE_MARGIN - EDGE_SATURATION)
        };
        p.clamp(0.0, 1.0)
    };
    if position < EDGE_MARGIN {
        -near(position.max(0.0))
    } else if position > size - EDGE_MARGIN {
        near((size - position).max(0.0))
    } else {
        0.0
    }
}

/// [`edge_direction`] for a listing that starts `top` points down and is `height` tall; 0 for
/// a point outside it.
pub fn band_direction(y: f32, top: f32, height: f32) -> f32 {
    if y < top || y >= top + height {
        return 0.0;
    }
    edge_direction(y - top, height)
}

/// The speed for `direction` (from [`edge_direction`]) after `dwell` in that direction: the
/// greatest times the square of the nearness, times a share that rises from
/// [`RAMP_FLOOR`] to 1 over [`RAMP`] on a smoothstep. Its sign is the direction's.
pub fn scroll_speed(direction: f32, dwell: Duration, max_speed: f32) -> f32 {
    let proximity = direction.abs().clamp(0.0, 1.0);
    if proximity.is_nan() || proximity <= 0.0 {
        return 0.0;
    }
    let ramp = (dwell.as_secs_f32() / RAMP.as_secs_f32()).clamp(0.0, 1.0);
    let share = RAMP_FLOOR + (1.0 - RAMP_FLOOR) * (3.0 * ramp * ramp - 2.0 * ramp * ramp * ramp);
    direction.signum() * max_speed * proximity * proximity * share
}
