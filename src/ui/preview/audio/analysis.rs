// SPDX-License-Identifier: MIT

use std::f32::consts::PI;

pub(super) const FFT_SIZE: usize = 2048;
const SAMPLE_RATE: f32 = crate::media::SAMPLE_RATE as f32;
const MIN_FREQUENCY: f32 = 40.0;
const MAX_FREQUENCY: f32 = 16_000.0;
const FLOOR_DB: f32 = -62.0;
const CEILING_DB: f32 = -6.0;
const BAR_FALL_PER_SECOND: f32 = 3.4;
const PEAK_HOLD_SECONDS: f32 = 0.42;
const PEAK_GRAVITY: f32 = 2.6;
const SETTLED_LEVEL: f32 = 0.002;

pub(super) struct Analyzer {
    window: Vec<f32>,
    twiddles: Vec<(f32, f32)>,
    reversed: Vec<usize>,
    real: Vec<f32>,
    imaginary: Vec<f32>,
    power: Vec<f32>,
}

impl Analyzer {
    pub(super) fn new() -> Self {
        let bits = FFT_SIZE.trailing_zeros();
        Self {
            window: (0..FFT_SIZE)
                .map(|index| 0.5 - 0.5 * (2.0 * PI * index as f32 / FFT_SIZE as f32).cos())
                .collect(),
            twiddles: (0..FFT_SIZE / 2)
                .map(|index| {
                    let angle = -2.0 * PI * index as f32 / FFT_SIZE as f32;
                    (angle.cos(), angle.sin())
                })
                .collect(),
            reversed: (0..FFT_SIZE)
                .map(|index| index.reverse_bits() >> (usize::BITS - bits))
                .collect(),
            real: vec![0.0; FFT_SIZE],
            imaginary: vec![0.0; FFT_SIZE],
            power: vec![0.0; FFT_SIZE / 2],
        }
    }

    /// Bin powers scaled so a full-scale sine peaks at 1.0.
    pub(super) fn power_spectrum(&mut self, samples: &[f32]) -> &[f32] {
        debug_assert_eq!(samples.len(), FFT_SIZE);
        for (index, &target) in self.reversed.iter().enumerate() {
            self.real[target] = samples[index] * self.window[index];
            self.imaginary[target] = 0.0;
        }
        let mut length = 2;
        while length <= FFT_SIZE {
            let half = length / 2;
            let stride = FFT_SIZE / length;
            for start in (0..FFT_SIZE).step_by(length) {
                for offset in 0..half {
                    let (cos, sin) = self.twiddles[offset * stride];
                    let even = start + offset;
                    let odd = even + half;
                    let real = self.real[odd] * cos - self.imaginary[odd] * sin;
                    let imaginary = self.real[odd] * sin + self.imaginary[odd] * cos;
                    self.real[odd] = self.real[even] - real;
                    self.imaginary[odd] = self.imaginary[even] - imaginary;
                    self.real[even] += real;
                    self.imaginary[even] += imaginary;
                }
            }
            length *= 2;
        }
        let scale = (4.0 / FFT_SIZE as f32).powi(2);
        for (bin, power) in self.power.iter_mut().enumerate() {
            *power = (self.real[bin].powi(2) + self.imaginary[bin].powi(2)) * scale;
        }
        &self.power
    }
}

/// Summing band power makes pink noise read flat on the log-frequency axis.
pub(super) fn band_levels(power: &[f32], levels: &mut [f32]) {
    let count = levels.len();
    if count == 0 || power.is_empty() {
        return;
    }
    let bins_per_hz = FFT_SIZE as f32 / SAMPLE_RATE;
    let ratio = MAX_FREQUENCY / MIN_FREQUENCY;
    let edge = |index: usize| MIN_FREQUENCY * ratio.powf(index as f32 / count as f32) * bins_per_hz;
    for (index, level) in levels.iter_mut().enumerate() {
        let (low, high) = (edge(index), edge(index + 1));
        let first = (low + 0.5).floor().max(0.0) as usize;
        let last = ((high + 0.5).ceil() as usize).min(power.len());
        let mut energy = 0.0;
        for (bin, value) in power.iter().enumerate().take(last).skip(first) {
            let overlap = (high.min(bin as f32 + 0.5) - low.max(bin as f32 - 0.5)).max(0.0);
            energy += value * overlap;
        }
        let decibels = 10.0 * (energy + 1e-12).log10();
        *level = ((decibels - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0);
    }
}

#[derive(Clone, Copy, Default)]
struct Peak {
    level: f32,
    hold: f32,
    velocity: f32,
}

#[derive(Default)]
pub(super) struct Ballistics {
    bars: Vec<f32>,
    peaks: Vec<Peak>,
}

impl Ballistics {
    pub(super) fn resize(&mut self, count: usize) {
        if self.bars.len() == count {
            return;
        }
        let previous = std::mem::take(&mut self.bars);
        let previous_peaks = std::mem::take(&mut self.peaks);
        let source = |index: usize| index * previous.len() / count.max(1);
        self.bars = (0..count)
            .map(|index| previous.get(source(index)).copied().unwrap_or(0.0))
            .collect();
        self.peaks = (0..count)
            .map(|index| {
                previous_peaks
                    .get(source(index))
                    .copied()
                    .unwrap_or_default()
            })
            .collect();
    }

    /// `elapsed` is in seconds; absent targets let the bars and peaks decay.
    pub(super) fn update(&mut self, targets: Option<&[f32]>, elapsed: f32) {
        let elapsed = elapsed.clamp(0.0, 0.25);
        for (index, (bar, peak)) in self.bars.iter_mut().zip(&mut self.peaks).enumerate() {
            let target = targets
                .and_then(|targets| targets.get(index))
                .copied()
                .unwrap_or(0.0);
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

    pub(super) fn bars(&self) -> &[f32] {
        &self.bars
    }

    pub(super) fn peaks(&self) -> impl Iterator<Item = f32> + '_ {
        self.peaks.iter().map(|peak| peak.level)
    }

    pub(super) fn is_settled(&self) -> bool {
        self.peaks.iter().all(|peak| peak.level <= SETTLED_LEVEL)
    }
}

#[cfg(test)]
mod tests;
