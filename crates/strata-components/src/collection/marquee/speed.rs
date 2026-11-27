// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::Spring;

/// Where edge auto-scroll starts and how fast it gets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeScroll {
    /// How far inside a viewport edge (points) auto-scroll starts: Strata's `AUTO_SCROLL_MARGIN`, 28.
    pub edge: f32,
    /// The speed at the edge and beyond, in points a second.
    pub max_speed: f32,
}

impl EdgeScroll {
    /// Strata's values.
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

/// The auto-scroll speed, in points a second.
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

/// How far ahead of the scroll offset to aim a glide on `spring`, retargeted every `step`.
pub fn glide_lead(speed: f32, spring: Spring, step: Duration) -> f32 {
    let omega = spring.omega();
    let a = omega * step.as_secs_f32();
    if a > 1e-4 {
        speed * (a.exp() - 1.0 + a) / (omega * a)
    } else {
        2.0 * speed / omega
    }
}

/// How far ahead of the scroll offset to aim a glide moving at `speed` on `spring` so it comes to rest without passing the target: `v/ω`, the shortest distance a critically damped spring stops in without overshoot.
pub fn stop_lead(speed: f32, spring: Spring) -> f32 {
    speed / spring.omega()
}
