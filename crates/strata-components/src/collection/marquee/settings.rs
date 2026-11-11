//! What the marquee machine reads on every step: the host gives it to the collection
//! (`Collection::settings`), and the machine reads it through its surface
//! (`Ctx::settings::<MarqueeSettings>()`) on each input.

use std::time::Duration;

use rune::Spring;

use super::speed::EdgeScroll;

/// The marquee machine's settings: plain data the host passes to the collection each time it
/// builds it (`Collection::settings`). Without one, the defaults.
///
/// Fields may be added, so a host outside this crate can't write the struct out: it starts
/// from the defaults and sets fields.
///
/// ```ignore
/// let mut marquee = MarqueeSettings::default();
/// marquee.animations = look.animations;
/// collection.settings(marquee)
/// ```
///
/// **Hosts set [`MarqueeSettings::animations`] from `Look::animations`**, the switch they
/// also pass (negated) to `Cx::reduced_motion`. Left on under reduced motion, every glide
/// step lands at once a whole lead ahead, and the edge scroll runs several times too fast.
///
/// Used by: C2, C3, C4, and hosts.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarqueeSettings {
    /// Where edge auto-scroll starts and how fast it gets.
    pub edge: EdgeScroll,
    /// The spring auto-scroll glides on. Rune's `GLIDE` by default, the spring of the
    /// collection element's `ScrollTo { glide: true }`.
    pub glide: Spring,
    /// The frame interval assumed for the first glide step; later steps use the interval
    /// measured between ticks. 1/60 s by default.
    pub frame: Duration,
    /// While the view scrolls, the band is applied to the selection at most this often, so
    /// the commits stay under Rune's scroll-commit warning (D1 in the module docs). 100 ms by
    /// default.
    pub apply_every: Duration,
    /// And at most once in this many frames (ticks) while the view scrolls: the warning counts
    /// frames, one commit in 4, whatever the frame rate. 6 by default, which 100 ms is at
    /// 60 Hz; below 60 Hz the frames are what hold the band back.
    pub apply_frames: u32,
    /// False when the user or the desktop turned animations off: the host sets it from
    /// `Look::animations` (see above). Rune then lands each glide at once, so the machine aims
    /// one frame's travel ahead instead of a glide's lead.
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
