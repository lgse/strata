// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn resample_bands_linear_empty_returns_zeros() {
    let empty: [f32; 0] = [];
    let resampled = resample_bands_linear(&empty, 84);
    assert_eq!(resampled.len(), 84);
    for v in resampled {
        assert_eq!(v, 0.0);
    }
}

#[test]
fn resample_bands_linear_spreads_across_target_count() {
    let bands = [0.0f32, 0.5f32, 1.0f32];
    let resampled = resample_bands_linear(&bands, 5);
    assert_eq!(resampled.len(), 5);
    assert!((resampled[0] - 0.0).abs() < 1e-4);
    assert!((resampled[1] - 0.25).abs() < 1e-4);
    assert!((resampled[2] - 0.5).abs() < 1e-4);
    assert!((resampled[3] - 0.75).abs() < 1e-4);
    assert!((resampled[4] - 1.0).abs() < 1e-4);
}

#[test]
fn resample_bands_linear_single_band() {
    let bands = [0.42f32];
    let resampled = resample_bands_linear(&bands, 10);
    assert_eq!(resampled.len(), 10);
    for v in resampled {
        assert!((v - 0.42).abs() < 1e-4);
    }
}
