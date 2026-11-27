// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::Spring;

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct AutoscrollSettings {
    pub dead_zone: f32,
    pub span: f32,
    pub max_speed: f32,
    /// The spring each frame's glide runs on: Rune's `GLIDE`, as the marquee's edge scroll.
    pub glide: Spring,
    pub frame: Duration,
    pub margin: f32,
}

impl AutoscrollSettings {
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

pub fn speed(delta: f32, s: &AutoscrollSettings) -> f32 {
    let travel = delta.abs() - s.dead_zone;
    // Written so that NaN fails the comparison and lands on zero.
    if !(travel > 0.0 && s.span > 0.0) {
        return 0.0;
    }
    let ratio = (travel / s.span).clamp(0.0, 1.0);
    delta.signum() * ratio * ratio * s.max_speed
}
