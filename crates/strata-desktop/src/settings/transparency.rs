// SPDX-License-Identifier: MIT

//! Persisted variable surface transparency & frosted glass preferences (`settings.toml`).
//!
//! Stored beside `Preferences` so Rune's transparency settings persist and merge through
//! [`super::Saving`] without altering GTK's `Preferences` schema.

use serde::{Deserialize, Serialize};

use super::{read_state_file, settings_path};

pub const MIN_SURFACE_TRANSPARENCY: u32 = 0;
pub const MAX_SURFACE_TRANSPARENCY: u32 = 100;
pub const DEFAULT_BACKGROUND_TRANSPARENCY: u32 = 15;
pub const DEFAULT_RAISED_SURFACE_TRANSPARENCY: u32 = 10;

pub const MIN_UNFOCUSED_TRANSPARENCY_MULTIPLIER: f64 = 0.0;
pub const MAX_UNFOCUSED_TRANSPARENCY_MULTIPLIER: f64 = 2.0;
pub const DEFAULT_UNFOCUSED_TRANSPARENCY_MULTIPLIER: f64 = 1.25;
pub const UNFOCUSED_TRANSPARENCY_MULTIPLIER_STEP: f64 = 0.05;

pub const TRANSPARENCY_ENABLED_KEY: &str = "transparency_enabled";
pub const BACKGROUND_TRANSPARENCY_KEY: &str = "background_transparency";
pub const RAISED_SURFACE_TRANSPARENCY_KEY: &str = "raised_surface_transparency";
pub const FROSTED_GLASS_KEY: &str = "frosted_glass";
pub const UNFOCUSED_TRANSPARENCY_MULTIPLIER_KEY: &str = "unfocused_transparency_multiplier";

/// Clamps a surface transparency percentage to `0..=100`.
pub const fn normalized_surface_transparency(percent: u32) -> u32 {
    if percent > MAX_SURFACE_TRANSPARENCY {
        MAX_SURFACE_TRANSPARENCY
    } else {
        percent
    }
}

/// Clamps and rounds the unfocused transparency multiplier to `0.00..=2.00` (0.01 precision).
/// Non-finite values fall back to [`DEFAULT_UNFOCUSED_TRANSPARENCY_MULTIPLIER`].
pub fn normalized_unfocused_transparency_multiplier(multiplier: f64) -> f64 {
    if !multiplier.is_finite() {
        return DEFAULT_UNFOCUSED_TRANSPARENCY_MULTIPLIER;
    }
    let clamped = multiplier.clamp(
        MIN_UNFOCUSED_TRANSPARENCY_MULTIPLIER,
        MAX_UNFOCUSED_TRANSPARENCY_MULTIPLIER,
    );
    (clamped * 100.0).round() / 100.0
}

const fn default_background_transparency() -> u32 {
    DEFAULT_BACKGROUND_TRANSPARENCY
}

const fn default_raised_surface_transparency() -> u32 {
    DEFAULT_RAISED_SURFACE_TRANSPARENCY
}

const fn default_frosted_glass() -> bool {
    true
}

const fn default_unfocused_multiplier() -> f64 {
    DEFAULT_UNFOCUSED_TRANSPARENCY_MULTIPLIER
}

/// Rune surface transparency & frosted glass settings stored in `settings.toml`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct TransparencyPreferences {
    /// Master switch; when `false`, all surfaces remain 100% opaque.
    #[serde(default, rename = "transparency_enabled")]
    pub enabled: bool,
    /// Transparency of the main background surfaces (`bg_base`, `page`), in percent `0..=100`.
    #[serde(default = "default_background_transparency")]
    pub background_transparency: u32,
    /// Independent transparency of raised chrome surfaces (sidebar, header, tabs, footer), `0..=100`.
    #[serde(default = "default_raised_surface_transparency")]
    pub raised_surface_transparency: u32,
    /// Whether to apply GPU backdrop blur and subtle frosted sheen on translucent chrome/overlays.
    #[serde(default = "default_frosted_glass")]
    pub frosted_glass: bool,
    /// Multiplier applied to transparency percentages when the window loses focus (`0.0..=2.0`).
    #[serde(
        default = "default_unfocused_multiplier",
        rename = "unfocused_transparency_multiplier"
    )]
    pub unfocused_multiplier: f64,
}

impl Default for TransparencyPreferences {
    fn default() -> Self {
        Self {
            enabled: false,
            background_transparency: DEFAULT_BACKGROUND_TRANSPARENCY,
            raised_surface_transparency: DEFAULT_RAISED_SURFACE_TRANSPARENCY,
            frosted_glass: true,
            unfocused_multiplier: DEFAULT_UNFOCUSED_TRANSPARENCY_MULTIPLIER,
        }
    }
}

impl TransparencyPreferences {
    /// Returns a copy with all numeric fields clamped into their accepted ranges.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Whether changing window focus affects the resolved surface alphas.
    pub fn varies_with_focus(self) -> bool {
        self.enabled && (self.unfocused_multiplier - 1.0).abs() > f64::EPSILON
    }

    /// Clamps numeric fields into their accepted ranges in place.
    pub fn normalize(&mut self) {
        self.background_transparency =
            normalized_surface_transparency(self.background_transparency);
        self.raised_surface_transparency =
            normalized_surface_transparency(self.raised_surface_transparency);
        self.unfocused_multiplier =
            normalized_unfocused_transparency_multiplier(self.unfocused_multiplier);
    }

    /// Writes all five keys into a `settings.toml` table snapshot.
    pub fn insert_into_table(self, table: &mut toml::Table) {
        let norm = self.normalized();
        table.insert(
            TRANSPARENCY_ENABLED_KEY.to_owned(),
            toml::Value::Boolean(norm.enabled),
        );
        table.insert(
            BACKGROUND_TRANSPARENCY_KEY.to_owned(),
            toml::Value::Integer(i64::from(norm.background_transparency)),
        );
        table.insert(
            RAISED_SURFACE_TRANSPARENCY_KEY.to_owned(),
            toml::Value::Integer(i64::from(norm.raised_surface_transparency)),
        );
        table.insert(
            FROSTED_GLASS_KEY.to_owned(),
            toml::Value::Boolean(norm.frosted_glass),
        );
        table.insert(
            UNFOCUSED_TRANSPARENCY_MULTIPLIER_KEY.to_owned(),
            toml::Value::Float(norm.unfocused_multiplier),
        );
    }
}

/// Parses transparency settings from raw `settings.toml` contents, salvaging each key
/// independently and clamping out-of-range values.
///
/// Returns `(preferences, repaired_keys)` where `repaired_keys` lists any keys whose stored
/// values had the wrong type or had to be clamped. Missing keys silently take defaults.
pub fn parse_transparency_preferences(
    contents: &str,
) -> (TransparencyPreferences, Vec<&'static str>) {
    let Ok(table) = toml::from_str::<toml::Table>(contents) else {
        return (TransparencyPreferences::default(), Vec::new());
    };
    let mut prefs = TransparencyPreferences::default();
    let mut repaired = Vec::new();

    parse_bool_field(
        &table,
        TRANSPARENCY_ENABLED_KEY,
        &mut prefs.enabled,
        &mut repaired,
    );
    parse_percent_field(
        &table,
        BACKGROUND_TRANSPARENCY_KEY,
        &mut prefs.background_transparency,
        &mut repaired,
    );
    parse_percent_field(
        &table,
        RAISED_SURFACE_TRANSPARENCY_KEY,
        &mut prefs.raised_surface_transparency,
        &mut repaired,
    );
    parse_bool_field(
        &table,
        FROSTED_GLASS_KEY,
        &mut prefs.frosted_glass,
        &mut repaired,
    );
    parse_multiplier_field(
        &table,
        UNFOCUSED_TRANSPARENCY_MULTIPLIER_KEY,
        &mut prefs.unfocused_multiplier,
        &mut repaired,
    );

    (prefs, repaired)
}

fn parse_bool_field(
    table: &toml::Table,
    key: &'static str,
    target: &mut bool,
    repaired: &mut Vec<&'static str>,
) {
    let Some(raw) = table.get(key) else {
        return;
    };
    match raw.as_bool() {
        Some(value) => *target = value,
        None => {
            tracing::warn!(
                key,
                "invalid boolean transparency preference; using default"
            );
            repaired.push(key);
        }
    }
}

fn parse_percent_field(
    table: &toml::Table,
    key: &'static str,
    target: &mut u32,
    repaired: &mut Vec<&'static str>,
) {
    let Some(raw) = table.get(key) else {
        return;
    };
    let Some(integer) = raw.as_integer() else {
        tracing::warn!(
            key,
            "invalid integer transparency preference; using default"
        );
        repaired.push(key);
        return;
    };
    if integer < 0 {
        *target = MIN_SURFACE_TRANSPARENCY;
        repaired.push(key);
        return;
    }
    let clamped = u32::try_from(integer)
        .map(normalized_surface_transparency)
        .unwrap_or(MAX_SURFACE_TRANSPARENCY);
    if i64::from(clamped) != integer {
        repaired.push(key);
    }
    *target = clamped;
}

fn parse_multiplier_field(
    table: &toml::Table,
    key: &'static str,
    target: &mut f64,
    repaired: &mut Vec<&'static str>,
) {
    let Some(raw) = table.get(key) else {
        return;
    };
    let Some(number) = raw
        .as_float()
        .or_else(|| raw.as_integer().map(|v| v as f64))
    else {
        tracing::warn!(
            key,
            "invalid numeric transparency multiplier; using default"
        );
        repaired.push(key);
        return;
    };
    let normalized = normalized_unfocused_transparency_multiplier(number);
    if !number.is_finite() || (normalized - number).abs() > f64::EPSILON {
        repaired.push(key);
    }
    *target = normalized;
}

/// Reads transparency preferences from `settings.toml`.
pub(super) fn read_saved_transparency() -> (TransparencyPreferences, Vec<&'static str>) {
    match read_state_file(&settings_path()) {
        Ok(Some(contents)) => parse_transparency_preferences(&contents),
        Ok(None) | Err(_) => (TransparencyPreferences::default(), Vec::new()),
    }
}

#[cfg(test)]
mod tests;
