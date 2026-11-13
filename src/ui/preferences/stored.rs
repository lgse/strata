// SPDX-License-Identifier: MIT

//! How settings are stored: string forms of modes, densities and sorts, and the ranges values are kept within.

use super::*;

pub(super) fn browser_mode_from_stored(value: &str) -> super::super::browser_modes::BrowserMode {
    match value {
        "icons" | "grid" => super::super::browser_modes::BrowserMode::Icons,
        "list" | "explorer" => super::super::browser_modes::BrowserMode::List,
        _ => super::super::browser_modes::BrowserMode::Columns,
    }
}

pub(super) fn stored_browser_mode(mode: super::super::browser_modes::BrowserMode) -> &'static str {
    match mode {
        super::super::browser_modes::BrowserMode::Columns => "columns",
        super::super::browser_modes::BrowserMode::Icons => "icons",
        super::super::browser_modes::BrowserMode::List => "list",
    }
}

/// `D`'s scale, as the shared density rules take it.
fn presets<D: DensityScale>() -> strata_desktop::settings::DensityPresets {
    strata_desktop::settings::DensityPresets {
        compact: D::COMPACT_PX,
        airy: D::AIRY_PX,
        max: D::MAX_PX,
    }
}

pub(super) fn stored_density<D: DensityScale>(mode: &str, custom_px: u8) -> D {
    D::from_px(strata_desktop::settings::stored_density_px(
        presets::<D>(),
        mode,
        custom_px,
    ))
}

pub(super) fn store_density<D: DensityScale>(density: D, mode: &mut String, custom_px: &mut u8) {
    strata_desktop::settings::store_density_px(presets::<D>(), density.px(), mode, custom_px);
}
