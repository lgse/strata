// SPDX-License-Identifier: MIT

//! Persisted visual effect preference for entering 10xer mode (`tenxer_fx` in `settings.toml`).

use serde::{Deserialize, Serialize};

use super::{read_state_file, settings_path};

/// GPU visual effect played when entering 10xer mode on Rune.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TenxerFx {
    #[default]
    HorizonScan,
    VelocityWarp,
    ShockwaveRing,
    PerimeterIgnite,
    TacticalHud,
    InterfaceOnFire,
    Off,
}

impl TenxerFx {
    /// Key stored in `settings.toml`.
    pub const KEY: &'static str = "tenxer_fx";

    /// Every selectable option in display order.
    pub const ALL: [Self; 7] = [
        Self::HorizonScan,
        Self::VelocityWarp,
        Self::ShockwaveRing,
        Self::PerimeterIgnite,
        Self::TacticalHud,
        Self::InterfaceOnFire,
        Self::Off,
    ];

    /// Canonical `snake_case` value written to `settings.toml`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HorizonScan => "horizon_scan",
            Self::VelocityWarp => "velocity_warp",
            Self::ShockwaveRing => "shockwave_ring",
            Self::PerimeterIgnite => "perimeter_ignite",
            Self::TacticalHud => "tactical_hud",
            Self::InterfaceOnFire => "interface_on_fire",
            Self::Off => "off",
        }
    }

    /// English catalog key for the Settings UI label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::HorizonScan => "Horizon Scan",
            Self::VelocityWarp => "Velocity Warp",
            Self::ShockwaveRing => "Shockwave Ring",
            Self::PerimeterIgnite => "Perimeter Ignite",
            Self::TacticalHud => "Tactical HUD",
            Self::InterfaceOnFire => "Interface on fire",
            Self::Off => "Off",
        }
    }

    /// Whether an activation animation should run for this choice.
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Converts to the TOML string written under [`Self::KEY`].
    pub fn to_toml_value(self) -> toml::Value {
        toml::Value::String(self.as_str().to_owned())
    }
}

/// Wrapper for serde default/round-trip parsing of the `tenxer_fx` key in a TOML document.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
struct StoredTenxerFx {
    #[serde(default)]
    tenxer_fx: TenxerFx,
}

/// Parses `tenxer_fx` from raw `settings.toml` contents.
///
/// Returns `(effect, repaired)`: a missing key silently yields `TenxerFx::default()` with
/// `repaired == false`; a present but invalid value salvages to `TenxerFx::default()` with
/// `repaired == true`.
pub fn parse_tenxer_fx(contents: &str) -> (TenxerFx, bool) {
    let Ok(table) = toml::from_str::<toml::Table>(contents) else {
        return (TenxerFx::default(), false);
    };
    let Some(raw) = table.get(TenxerFx::KEY) else {
        return (TenxerFx::default(), false);
    };
    let mut single = toml::Table::new();
    single.insert(TenxerFx::KEY.to_owned(), raw.clone());
    match single.try_into::<StoredTenxerFx>() {
        Ok(stored) => (stored.tenxer_fx, false),
        Err(error) => {
            tracing::warn!(%error, "invalid tenxer_fx preference; falling back to default");
            (TenxerFx::default(), true)
        }
    }
}

/// Reads `tenxer_fx` from `settings.toml`, returning `(effect, repaired)`.
pub(super) fn read_saved_tenxer_fx() -> (TenxerFx, bool) {
    match read_state_file(&settings_path()) {
        Ok(Some(contents)) => parse_tenxer_fx(&contents),
        Ok(None) | Err(_) => (TenxerFx::default(), false),
    }
}

#[cfg(test)]
mod tests;
