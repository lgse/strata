//! The waveform's levels and their grow-in (Strata's `src/ui/preview/waveform.rs`).

/// Below this the loudest level counts as this, so a silent track doesn't blow up noise.
const MIN_LOUDEST: f32 = 0.04;
/// Bars approach their targets at this rate, per second (an exponential approach).
const GROW_RATE: f32 = 9.0;
/// A bar this close to its target lands on it.
const SETTLED: f32 = 0.002;

/// The waveform's levels: what the host sent (targets) and what's drawn (shown).
#[derive(Clone, Debug, Default)]
pub struct Levels {
    targets: Vec<f32>,
    shown: Vec<f32>,
    loudest: f32,
    /// The levels are being cleared: the next ones start a fresh scale.
    stale: bool,
    /// Bumps each time the shown levels change, so the view knows to rebuild.
    generation: u64,
}

impl Levels {
    /// Levels from `start`, RMS from 0 to 1, into `buckets` buckets. A new bucket count
    /// starts over.
    pub fn set(&mut self, buckets: usize, start: usize, levels: &[f32]) {
        if self.targets.len() != buckets {
            self.targets = vec![0.0; buckets];
            self.shown = vec![0.0; buckets];
            self.loudest = 0.0;
        }
        if std::mem::take(&mut self.stale) {
            self.loudest = 0.0;
        }
        for (t, l) in self.targets.iter_mut().skip(start).zip(levels) {
            *t = l.clamp(0.0, 1.0);
            self.loudest = self.loudest.max(*t);
        }
    }

    /// The outgoing waveform lowers, keeping its scale while it does.
    pub fn clear(&mut self) {
        self.targets.iter_mut().for_each(|t| *t = 0.0);
        self.stale = true;
    }

    /// Whether any level was ever given (else the timeline is a plain line).
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Whether the shown levels still move toward their targets.
    pub fn moving(&self) -> bool {
        self.shown.iter().zip(&self.targets).any(|(s, t)| s != t)
    }

    /// One step of the grow-in, over `elapsed` seconds; with motion off they land at once.
    /// True when the shown levels changed.
    pub fn step(&mut self, elapsed: f32, motion: bool) -> bool {
        if !self.moving() {
            return false;
        }
        if !motion {
            self.shown.clone_from(&self.targets);
        } else {
            let k = 1.0 - (-GROW_RATE * elapsed.clamp(0.0, 0.1)).exp();
            for (s, t) in self.shown.iter_mut().zip(&self.targets) {
                *s += (t - *s) * k;
                if (t - *s).abs() <= SETTLED {
                    *s = *t;
                }
            }
        }
        self.generation += 1;
        true
    }

    /// The level each of `count` bars draws, from 0 to 1 of the loudest: each bar the
    /// loudest of its buckets (`Waveform::draw`).
    pub fn bars(&self, count: usize) -> Vec<f32> {
        let n = self.shown.len();
        if n == 0 || count == 0 {
            return vec![0.0; count];
        }
        let loudest = self.loudest.max(MIN_LOUDEST);
        (0..count)
            .map(|i| {
                let first = i * n / count;
                let last = ((i + 1) * n / count).max(first + 1).min(n);
                let level = self.shown[first..last].iter().copied().fold(0.0, f32::max);
                (level / loudest).min(1.0)
            })
            .collect()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}
