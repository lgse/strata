//! Edge auto-scroll: how fast the view scrolls for a pointer near a viewport edge, and the
//! glide targets that reach and hold that speed on a Rune spring. Pure maths; speeds are in
//! points a second and times in [`Duration`]s.

use std::time::Duration;

use rune::Spring;

/// Where edge auto-scroll starts and how fast it gets.
///
/// Used by: C1d, C2 (its column strip), and hosts that tune it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeScroll {
    /// How far inside a viewport edge (points) auto-scroll starts: Strata's
    /// `AUTO_SCROLL_MARGIN`, 28.
    pub edge: f32,
    /// The speed at the edge and beyond, in points a second: Strata's `AUTO_SCROLL_MAX_STEP` of
    /// 24 pt per 16 ms tick (`AUTO_SCROLL_INTERVAL`), 1,500.
    pub max_speed: f32,
}

impl EdgeScroll {
    /// Strata's values (`src/ui/marquee/geometry.rs`, `upstream/main` 35ee677ab).
    pub const STRATA: EdgeScroll = EdgeScroll {
        edge: 28.0,
        max_speed: 1500.0,
    };
}

impl Default for EdgeScroll {
    fn default() -> Self {
        Self::STRATA
    }
}

/// The auto-scroll speed, in points a second, for a pointer `pos` points from the start of a
/// viewport `size` points long: negative scrolls towards the start, positive towards the end,
/// zero away from the edges. It grows linearly from zero at [`EdgeScroll::edge`] inside an
/// edge to [`EdgeScroll::max_speed`] at the edge, and stays there beyond it, as Strata's
/// `auto_scroll_step` does. A viewport no longer than two edge bands never scrolls, and nor
/// does a NaN position.
pub fn edge_speed(pos: f32, size: f32, e: EdgeScroll) -> f32 {
    // Written so that NaN fails every comparison and lands on zero.
    if !(e.edge > 0.0 && size > 2.0 * e.edge) {
        return 0.0;
    }
    let overshoot = if pos < e.edge {
        pos - e.edge
    } else if pos > size - e.edge {
        pos - (size - e.edge)
    } else {
        return 0.0;
    };
    (overshoot / e.edge).clamp(-1.0, 1.0) * e.max_speed
}

/// How far ahead of the scroll offset to aim a glide on `spring`, retargeted every `step`, so
/// that it settles at `speed` points a second.
///
/// Rune keeps a glide's speed when a request retargets it (`retarget_v`) and steps it on the
/// spring's exact critically damped solution. A target held a constant `L` ahead settles where
/// one step neither speeds the glide up nor slows it down: `v = ω·L·a / (eᵃ − 1 + a)` with
/// `a = ω·step`, so `L = v·(eᵃ − 1 + a) / (ω·a)`. That tends to `2v/ω` as the step shrinks.
/// Probed at the pinned Rune rev with `GLIDE` at 60 Hz: within 1 % of the speed asked, 90 % of
/// it after 0.07 s; the continuous `2v/ω` lead fell 7.6 % short.
pub fn glide_lead(speed: f32, spring: Spring, step: Duration) -> f32 {
    let omega = spring.omega();
    let a = omega * step.as_secs_f32();
    if a > 1e-4 {
        speed * (a.exp() - 1.0 + a) / (omega * a)
    } else {
        2.0 * speed / omega
    }
}

/// How far ahead of the scroll offset to aim a glide moving at `speed` on `spring` so it comes
/// to rest without passing the target: `v/ω`, the shortest distance a critically damped spring
/// stops in without overshoot. A slower glide (one still speeding up) stops short of it, also
/// without overshoot. Probed at the pinned Rune rev: no overshoot, at rest within 0.3 s.
pub fn stop_lead(speed: f32, spring: Spring) -> f32 {
    speed / spring.omega()
}
