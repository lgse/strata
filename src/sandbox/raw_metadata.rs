// SPDX-License-Identifier: MIT

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::metadata::MAX_METADATA_BYTES;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct RawMetadata {
    pub(crate) dimensions: Option<(u32, u32)>,
    pub(crate) camera: Option<String>,
    pub(crate) lens: Option<String>,
    pub(crate) focal_length: Option<f64>,
    pub(crate) shutter_speed: Option<f64>,
    pub(crate) iso: Option<f64>,
    pub(crate) gps: Option<(f64, f64)>,
}

impl RawMetadata {
    pub(crate) fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_METADATA_BYTES {
            return Err("RAW metadata exceeds the size limit".into());
        }
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if !value.is_object() {
            return Err("Invalid RAW metadata object".into());
        }
        let mut metadata: Self =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        metadata.dimensions = metadata.dimensions.filter(|(width, height)| {
            (1..=1_000_000).contains(width) && (1..=1_000_000).contains(height)
        });
        metadata.camera = metadata.camera.filter(|value| valid_text(value));
        metadata.lens = metadata.lens.filter(|value| valid_text(value));
        metadata.focal_length = positive(metadata.focal_length, 100_000.0);
        metadata.shutter_speed =
            positive(metadata.shutter_speed, 604_800.0).filter(|value| *value >= 1e-9);
        metadata.iso = positive(metadata.iso, 100_000_000.0);
        metadata.gps = metadata.gps.filter(|(latitude, longitude)| {
            latitude.is_finite()
                && longitude.is_finite()
                && (-90.0..=90.0).contains(latitude)
                && (-180.0..=180.0).contains(longitude)
        });
        Ok(metadata)
    }
}

fn positive(value: Option<f64>, maximum: f64) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0 && *value <= maximum)
}

fn valid_text(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 256
        && !value.chars().any(|character| {
            character.is_control()
                || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

pub(crate) fn is_raw(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "3fr"
                    | "arw"
                    | "cr2"
                    | "cr3"
                    | "dcr"
                    | "dng"
                    | "erf"
                    | "kdc"
                    | "mef"
                    | "mos"
                    | "mrw"
                    | "nef"
                    | "nrw"
                    | "orf"
                    | "pef"
                    | "raf"
                    | "raw"
                    | "rw2"
                    | "rwl"
                    | "sr2"
                    | "srf"
                    | "srw"
                    | "x3f"
            )
        })
}

#[cfg(test)]
mod tests;
