// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn silent_samples_produce_minimal_bands() {
    let silent = vec![0u8; 4096];
    let bands = compute_spectrum_bands(&silent);
    assert_eq!(bands.len(), 24);
    for &b in &bands {
        assert!(b < 0.05, "Silence should produce minimal energy, got {b}");
    }
}

#[test]
fn low_frequency_tone_excites_bass_bands() {
    // Generate a 100 Hz sine wave at 48 kHz
    let mut samples = Vec::with_capacity(4096);
    for k in 0..1024 {
        let t = k as f32 / 48000.0;
        let val = (2.0 * std::f32::consts::PI * 100.0 * t).sin();
        let s = (val * 30000.0) as i16;
        samples.extend_from_slice(&s.to_le_bytes()); // Left
        samples.extend_from_slice(&s.to_le_bytes()); // Right
    }
    let bands = compute_spectrum_bands(&samples);
    // 100 Hz falls in the lower log bands (around bands 3..7)
    let bass_energy: f32 = bands[2..8].iter().copied().fold(0.0, f32::max);
    let treble_energy: f32 = bands[18..24].iter().copied().fold(0.0, f32::max);
    assert!(
        bass_energy > 0.4,
        "100 Hz tone should excite lower bands, got {bass_energy}"
    );
    assert!(
        bass_energy > treble_energy,
        "Bass energy ({bass_energy}) should exceed treble energy ({treble_energy})"
    );
}

#[test]
fn high_frequency_tone_excites_treble_bands() {
    // Generate a 6000 Hz sine wave at 48 kHz
    let mut samples = Vec::with_capacity(4096);
    for k in 0..1024 {
        let t = k as f32 / 48000.0;
        let val = (2.0 * std::f32::consts::PI * 6000.0 * t).sin();
        let s = (val * 30000.0) as i16;
        samples.extend_from_slice(&s.to_le_bytes()); // Left
        samples.extend_from_slice(&s.to_le_bytes()); // Right
    }
    let bands = compute_spectrum_bands(&samples);
    // 6000 Hz falls in the upper log bands (around bands 18..22)
    let treble_energy: f32 = bands[18..23].iter().copied().fold(0.0, f32::max);
    let sub_bass_energy: f32 = bands[0..4].iter().copied().fold(0.0, f32::max);
    assert!(
        treble_energy > 0.4,
        "6000 Hz tone should excite treble bands, got {treble_energy}"
    );
    assert!(
        treble_energy > sub_bass_energy,
        "Treble energy ({treble_energy}) should exceed sub-bass energy ({sub_bass_energy})"
    );
}
