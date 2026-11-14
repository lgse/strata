// SPDX-License-Identifier: MIT

use super::{DensityPresets, store_density_px, stored_density_px};

const CONTENT: DensityPresets = DensityPresets {
    compact: 4,
    airy: 8,
    max: 16,
};

/// A value saved is read back as itself, a preset by its name and any other value as Custom;
/// a value past the end saves as the end, and a preset keeps the last Custom px.
#[test]
fn a_density_reads_back_as_it_was_saved() {
    let mut mode = String::new();
    let mut custom = 11;
    for (px, saved_mode, saved_custom, read) in [
        (4, "compact", 11, 4),
        (8, "airy", 11, 8),
        (6, "custom", 6, 6),
        (0, "custom", 0, 0),
        (40, "custom", 16, 16),
        (8, "airy", 16, 8),
    ] {
        store_density_px(CONTENT, px, &mut mode, &mut custom);
        assert_eq!((mode.as_str(), custom), (saved_mode, saved_custom), "{px}");
        assert_eq!(stored_density_px(CONTENT, &mode, custom), read, "{px}");
    }
}

/// What a hand-edited file says: an unknown mode is Compact, a Custom value past the end is
/// the end.
#[test]
fn an_unknown_mode_reads_as_compact_and_a_custom_value_is_clamped() {
    assert_eq!(stored_density_px(CONTENT, "roomy", 9), 4);
    assert_eq!(stored_density_px(CONTENT, "custom", 200), 16);
}
