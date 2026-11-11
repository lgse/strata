//! The live spectrum: bar ballistics over the host's band levels, drawn through `Live`
//! values with no view commit (Strata's `src/ui/preview/audio/spectrum.rs` and `analysis.rs`).
//!
//! The host analyses the audio being heard (Strata's FFT and `band_levels`) and feeds the
//! band levels, 0 to 1, in any number of bands ([`Spectrum::feed`]). Each tick the bars rise to
//! their band at once and fall at 3.4 a second; a peak cap holds 0.42 s, then falls under
//! gravity. With no feed (paused, nothing heard) they fall to rest.

use std::cell::Cell;

use rune::{Color, Live};

const BAR_FALL_PER_SECOND: f32 = 3.4;
const PEAK_HOLD_SECONDS: f32 = 0.42;
const PEAK_GRAVITY: f32 = 2.6;
const SETTLED_LEVEL: f32 = 0.002;
/// Strata's bar count limits.
pub const MIN_BARS: usize = 12;
pub const MAX_BARS: usize = 96;
/// The share of the height under the baseline, for the reflection.
pub const REFLECTION: f32 = 0.2;
/// A bar at rest is this tall, in points.
pub const REST_HEIGHT: f32 = 2.0;
/// A peak cap's height.
pub const PEAK_HEIGHT: f32 = 2.0;
/// Caps lower than this aren't drawn.
const PEAK_SHOWN: f32 = 0.015;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Peak {
    level: f32,
    hold: f32,
    velocity: f32,
}

/// Bar levels and their peak caps (`Ballistics`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ballistics {
    bars: Vec<f32>,
    peaks: Vec<Peak>,
}

impl Ballistics {
    /// `count` bars, carrying the old bars' motion over.
    pub fn resize(&mut self, count: usize) {
        if self.bars.len() == count {
            return;
        }
        let (old, old_peaks) = (
            std::mem::take(&mut self.bars),
            std::mem::take(&mut self.peaks),
        );
        let source = |i: usize| i * old.len() / count.max(1);
        self.bars = (0..count)
            .map(|i| old.get(source(i)).copied().unwrap_or(0.0))
            .collect();
        self.peaks = (0..count)
            .map(|i| old_peaks.get(source(i)).copied().unwrap_or_default())
            .collect();
    }

    /// `elapsed` seconds on; absent targets let bars and peaks decay.
    pub fn update(&mut self, targets: Option<&[f32]>, elapsed: f32) {
        let elapsed = elapsed.clamp(0.0, 0.25);
        for (i, (bar, peak)) in self.bars.iter_mut().zip(&mut self.peaks).enumerate() {
            let target = targets.and_then(|t| t.get(i)).copied().unwrap_or(0.0);
            *bar = if target >= *bar {
                target
            } else {
                (*bar - BAR_FALL_PER_SECOND * elapsed).max(target)
            };
            if *bar >= peak.level {
                *peak = Peak {
                    level: *bar,
                    hold: PEAK_HOLD_SECONDS,
                    velocity: 0.0,
                };
            } else if peak.hold > 0.0 {
                peak.hold -= elapsed;
            } else {
                peak.velocity += PEAK_GRAVITY * elapsed;
                peak.level = (peak.level - peak.velocity * elapsed).max(*bar);
            }
        }
    }

    pub fn bars(&self) -> &[f32] {
        &self.bars
    }

    pub fn peaks(&self) -> impl Iterator<Item = f32> + '_ {
        self.peaks.iter().map(|p| p.level)
    }

    /// Every cap is down.
    pub fn is_settled(&self) -> bool {
        self.peaks.iter().all(|p| p.level <= SETTLED_LEVEL)
    }
}

/// One bar's live values.
#[derive(Clone)]
pub(crate) struct BarLive {
    pub(crate) bar: Live<[f32; 4]>,
    pub(crate) top: Live<Color>,
    pub(crate) peak: Live<[f32; 4]>,
    pub(crate) reflection: Live<[f32; 4]>,
}

/// The spectrum: ballistics, the latest feed, and the bars' live values.
#[derive(Clone, Default)]
pub struct Spectrum {
    ballistics: Ballistics,
    fed: Option<Vec<f32>>,
    pub(crate) bars: Vec<BarLive>,
    /// The spectrum's size in points, from its layout.
    size: [f32; 2],
    /// The bar colours at the build that made the live values (base, bright).
    colors: Cell<(Color, Color)>,
    gap: f32,
}

/// How many bars a width holds (`bar_count`), within Strata's limits.
pub fn bar_count(width: f32, bar: f32, gap: f32) -> usize {
    (((width + gap) / (bar + gap)).floor().max(0.0) as usize).clamp(MIN_BARS, MAX_BARS)
}

impl Spectrum {
    /// Band levels heard since the last tick, 0 to 1, in any number of bands. Each bar takes
    /// the loudest of the bands it spans.
    pub fn feed(&mut self, bands: &[f32]) {
        self.fed = Some(bands.to_vec());
    }

    pub fn ballistics(&self) -> &Ballistics {
        &self.ballistics
    }

    /// The spectrum's laid-out size, and the bar count the build uses. True when the count
    /// changed (the view rebuilds with that many bars).
    pub(crate) fn set_size(&mut self, size: [f32; 2], bar: f32, gap: f32) -> bool {
        self.size = size;
        self.gap = gap;
        let count = bar_count(size[0], bar, gap);
        let changed = self.bars.len() != count;
        if changed {
            self.ballistics.resize(count);
            self.bars = (0..count)
                .map(|_| BarLive {
                    bar: Live::new([0.0, 0.0, 0.0, 0.0]),
                    top: Live::new(self.colors.get().0),
                    peak: Live::new([0.0, 0.0, 0.0, 0.0]),
                    reflection: Live::new([0.0, 0.0, 0.0, 0.0]),
                })
                .collect();
        }
        self.place();
        changed
    }

    /// The build's bar colours: a theme change recolours the bars.
    pub(crate) fn set_colors(&self, base: Color, bright: Color) {
        if self.colors.get() != (base, bright) {
            self.colors.set((base, bright));
            self.place();
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.bars.len()
    }

    /// One tick, `elapsed` seconds on. True while anything still moves.
    pub(crate) fn tick(&mut self, elapsed: f32) -> bool {
        let fed = self.fed.take();
        let count = self.bars.len();
        let targets = fed.map(|bands| map_bands(&bands, count));
        self.ballistics.update(targets.as_deref(), elapsed);
        self.place();
        targets.is_some() || !self.is_settled()
    }

    pub(crate) fn is_settled(&self) -> bool {
        self.ballistics.is_settled() && self.ballistics.bars().iter().all(|b| *b <= SETTLED_LEVEL)
    }

    /// Writes the bars' rects and colours (`spectrum::draw`): every bar spans one gradient from
    /// the base at the baseline to the bright colour at the top, so a taller bar reaches a
    /// brighter colour.
    fn place(&self) {
        let [w, h] = self.size;
        let n = self.bars.len();
        if n == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let bar_w = ((w - self.gap * (n as f32 - 1.0)) / n as f32).max(1.0);
        let pitch = bar_w + self.gap;
        let baseline = (h * (1.0 - REFLECTION)).floor();
        let reflection_top = baseline + 2.0;
        let reflection_h = (h - reflection_top).max(0.0);
        let (base, bright) = self.colors.get();
        for (i, ((live, level), peak)) in self
            .bars
            .iter()
            .zip(self.ballistics.bars())
            .zip(self.ballistics.peaks())
            .enumerate()
        {
            let x = i as f32 * pitch;
            let bar = (level * baseline).max(REST_HEIGHT);
            live.bar.set([x, baseline - bar, bar_w, bar]);
            live.top
                .set(base.mix(bright, (bar / baseline).clamp(0.0, 1.0)));
            let r = (bar * 0.4).min(reflection_h);
            live.reflection
                .set([x, reflection_top, bar_w, if r > 1.0 { r } else { 0.0 }]);
            let top = (baseline - peak * baseline - PEAK_HEIGHT - 1.0).max(0.0);
            let shown = if peak > PEAK_SHOWN { PEAK_HEIGHT } else { 0.0 };
            live.peak.set([x, top, bar_w, shown]);
        }
    }
}

/// `bands` spread over `count` bars, each the loudest band it spans.
fn map_bands(bands: &[f32], count: usize) -> Vec<f32> {
    let n = bands.len();
    if n == 0 {
        return vec![0.0; count];
    }
    (0..count)
        .map(|i| {
            let first = i * n / count;
            let last = ((i + 1) * n / count).max(first + 1).min(n);
            bands[first..last]
                .iter()
                .copied()
                .fold(0.0, f32::max)
                .clamp(0.0, 1.0)
        })
        .collect()
}
