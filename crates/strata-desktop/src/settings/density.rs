// SPDX-License-Identifier: MIT

//! Frame and Content density as saved: a preset's name (`compact` or `airy`), or `custom`
//! with its px in the matching `*_custom_px` field. Both interfaces read and save them here,
//! the GTK one through `stored_density` and `store_density` in `src/ui/preferences/stored.rs`.
//! Each interface keeps its own scales, since the GTK and Rune sides draw with their own
//! types; they hold the same values.

#[cfg(test)]
mod tests;

/// A density's scale in px: its two presets and its end. Every value from 0 to `max` is
/// allowed; one on a preset saves as that preset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DensityPresets {
    pub compact: u8,
    pub airy: u8,
    pub max: u8,
}

/// The px the saved `mode` and `custom_px` stand for. An unknown mode reads as Compact, and a
/// Custom value past the end as the end.
pub fn stored_density_px(presets: DensityPresets, mode: &str, custom_px: u8) -> u8 {
    match mode {
        "airy" => presets.airy,
        "custom" => custom_px.min(presets.max),
        _ => presets.compact,
    }
}

/// Saves `px`, clamped to the scale, as `mode` and `custom_px`: a preset by its name, and any
/// other value as `custom` with its px. `custom_px` is left alone for a preset, so the last
/// Custom value is kept.
pub fn store_density_px(presets: DensityPresets, px: u8, mode: &mut String, custom_px: &mut u8) {
    let px = px.min(presets.max);
    *mode = if px == presets.compact {
        "compact"
    } else if px == presets.airy {
        "airy"
    } else {
        *custom_px = px;
        "custom"
    }
    .to_owned();
}
