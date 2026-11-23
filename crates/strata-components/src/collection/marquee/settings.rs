// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::Spring;

use super::speed::EdgeScroll;

/// The marquee machine's settings: plain data the host passes to the collection each time it builds it (`Collection::settings`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarqueeSettings {
    /// Where edge auto-scroll starts and how fast it gets.
    pub edge: EdgeScroll,
    /// The spring auto-scroll glides on.
    pub glide: Spring,
    /// The frame interval assumed for the first glide step; later steps use the interval measured between ticks.
    pub frame: Duration,
    /// While the view scrolls, the band is applied to the selection at most this often.
    pub apply_every: Duration,
    /// And at most once in this many frames (ticks) while the view scrolls.
    pub apply_frames: u32,
    /// False when the user or the desktop turned animations off: the host sets it from `Look::animations` (see above).
    pub animations: bool,
}

impl Default for MarqueeSettings {
    fn default() -> Self {
        Self {
            edge: EdgeScroll::STRATA,
            glide: rune::tok::motion::GLIDE,
            frame: Duration::from_micros(16_667),
            apply_every: Duration::from_millis(100),
            apply_frames: 6,
            animations: true,
        }
    }
}
