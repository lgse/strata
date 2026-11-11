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

pub(super) fn stored_density<D: DensityScale>(mode: &str, custom_px: u8) -> D {
    match mode {
        "airy" => D::from_px(D::AIRY_PX),
        "custom" => D::from_px(custom_px),
        _ => D::from_px(D::COMPACT_PX),
    }
}

pub(super) fn store_density<D: DensityScale>(density: D, mode: &mut String, custom_px: &mut u8) {
    let px = density.px();
    *mode = if px == D::COMPACT_PX {
        "compact"
    } else if px == D::AIRY_PX {
        "airy"
    } else {
        *custom_px = px;
        "custom"
    }
    .to_owned();
}
