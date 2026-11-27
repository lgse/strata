// SPDX-License-Identifier: MIT

/// The shortest gap between two measured samples, in seconds (Strata's 250 ms).
pub const SAMPLE_EVERY: f64 = 0.25;

pub const SMOOTHING: f64 = 0.4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RateMeter {
    last: Option<(f64, u64)>,
    rate: Option<f64>,
}

impl RateMeter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes `bytes` transferred so far at `now` seconds. Zero bytes are no sample.
    pub fn sample(&mut self, bytes: u64, now: f64) {
        if bytes == 0 {
            return;
        }
        match self.last {
            None => self.last = Some((now, bytes)),
            Some((at, before)) if bytes > before && now - at >= SAMPLE_EVERY => {
                let measured = (bytes - before) as f64 / (now - at);
                self.rate = Some(match self.rate {
                    Some(old) => old * (1.0 - SMOOTHING) + measured * SMOOTHING,
                    None => measured,
                });
                self.last = Some((now, bytes));
            }
            Some(_) => {}
        }
    }

    /// The smoothed rate, `None` until two samples far enough apart have measured one.
    pub fn rate(&self) -> Option<f64> {
        self.rate
    }
}
