// SPDX-License-Identifier: MIT

use super::value::Srgba;

pub const RECENT_MAX: usize = 8;

/// One colour to pick, with its accessible name. Without a label it is named by its hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Swatch {
    pub color: Srgba,
    pub label: Option<String>,
}

impl Swatch {
    pub fn new(color: Srgba) -> Swatch {
        Swatch { color, label: None }
    }

    /// Named `label` ("Accent", "Red").
    pub fn label(mut self, label: impl Into<String>) -> Swatch {
        self.label = Some(label.into());
        self
    }
}

/// A row of swatches under a heading ("Theme", "Recent"), heard as a radio group named by it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwatchRow {
    pub label: String,
    pub swatches: Vec<Swatch>,
}

impl SwatchRow {
    pub fn new(label: impl Into<String>, swatches: Vec<Swatch>) -> SwatchRow {
        SwatchRow {
            label: label.into(),
            swatches,
        }
    }
}

pub fn remember(recent: &mut Vec<Srgba>, color: Srgba, max: usize) {
    recent.retain(|&c| c != color);
    recent.insert(0, color);
    recent.truncate(max);
}
