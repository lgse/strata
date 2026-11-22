// SPDX-License-Identifier: MIT

//! Pure-Rust 2048-point Hann-windowed radix-2 FFT and log-frequency band integration
//! (40 Hz–16 kHz, -62 dB to -6 dB), matching GTK's `src/ui/preview/audio/analysis.rs`.

use std::f32::consts::PI;

pub(super) const FFT_SIZE: usize = 2048;
const SAMPLE_RATE: f32 = strata_preview::media::SAMPLE_RATE as f32;
const MIN_FREQUENCY: f32 = 40.0;
const MAX_FREQUENCY: f32 = 16_000.0;
const FLOOR_DB: f32 = -62.0;
const CEILING_DB: f32 = -6.0;

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

/// Reusable scratch buffers for feeding live spectrum levels per frame without reallocating.
pub(super) struct SpectrumFeed {
    analyzer: Analyzer,
    window: Vec<f32>,
    levels: Vec<f32>,
}

impl SpectrumFeed {
    pub(super) fn new() -> Self {
        Self {
            analyzer: Analyzer::new(),
            window: vec![0.0; FFT_SIZE],
            levels: Vec::new(),
        }
    }

    pub(super) fn window_mut(&mut self) -> &mut [f32] {
        &mut self.window
    }

    pub(super) fn analyze(&mut self, bands: usize) -> &[f32] {
        self.levels.resize(bands, 0.0);
        let power = self.analyzer.power_spectrum(&self.window);
        band_levels(power, &mut self.levels);
        &self.levels
    }
}

#[cfg(test)]
mod tests;
