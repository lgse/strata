// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DensityPresets {
    pub compact: u8,
    pub airy: u8,
    pub max: u8,
}

pub fn stored_density_px(presets: DensityPresets, mode: &str, custom_px: u8) -> u8 {
    match mode {
        "airy" => presets.airy,
        "custom" => custom_px.min(presets.max),
        _ => presets.compact,
    }
}

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
