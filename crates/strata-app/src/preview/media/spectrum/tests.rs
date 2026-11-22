// SPDX-License-Identifier: MIT

use super::*;

fn sine(frequency: f32, amplitude: f32) -> Vec<f32> {
    (0..FFT_SIZE)
        .map(|index| amplitude * (2.0 * PI * frequency * index as f32 / SAMPLE_RATE).sin())
        .collect()
}

fn levels_for(samples: &[f32], bars: usize) -> Vec<f32> {
    let mut analyzer = Analyzer::new();
    let mut levels = vec![0.0; bars];
    band_levels(analyzer.power_spectrum(samples), &mut levels);
    levels
}

fn loudest(levels: &[f32]) -> usize {
    levels
        .iter()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(right.1))
        .map(|(index, _)| index)
        .expect("a loudest bar")
}

fn bar_for(frequency: f32, bars: usize) -> usize {
    ((frequency / MIN_FREQUENCY).ln() / (MAX_FREQUENCY / MIN_FREQUENCY).ln() * bars as f32) as usize
}

#[test]
fn silence_draws_no_bars() {
    assert!(
        levels_for(&vec![0.0; FFT_SIZE], 48)
            .iter()
            .all(|level| *level == 0.0)
    );
}

#[test]
fn tones_light_the_bar_for_their_frequency() {
    for frequency in [60.0, 440.0, 1_000.0, 8_000.0] {
        let levels = levels_for(&sine(frequency, 0.5), 48);
        let expected = bar_for(frequency, 48);
        let peak = loudest(&levels);
        assert!(
            peak.abs_diff(expected) <= 1,
            "{frequency} Hz peaked at bar {peak}, expected {expected}"
        );
        assert!(levels[peak] > 0.8, "{frequency} Hz is too quiet");
        let distant = if expected > 24 { 0 } else { 47 };
        assert!(
            levels[distant] < 0.2,
            "{frequency} Hz leaked into bar {distant}"
        );
    }
}

#[test]
fn quieter_signals_draw_shorter_bars() {
    let loud = levels_for(&sine(1_000.0, 0.5), 32);
    let quiet = levels_for(&sine(1_000.0, 0.02), 32);
    let bar = loudest(&loud);
    assert!(quiet[bar] > 0.0 && quiet[bar] < loud[bar] - 0.3);
}
