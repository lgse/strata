//! Swatches as data: the rows the host passes (theme colours, recent colours) and the helper
//! that keeps a recent-colours list.

use super::value::Srgba;

/// How many recent colours GTK's chooser keeps (`gtkcolorchooserwidget.c`, custom colours).
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

/// Puts `color` first in `recent`, dropping an earlier copy of it and anything past `max`, as
/// GTK's chooser keeps its custom colours. The host stores the list and calls this when a
/// picker closes having changed its colour (`ColorPicker::on_close`).
pub fn remember(recent: &mut Vec<Srgba>, color: Srgba, max: usize) {
    recent.retain(|&c| c != color);
    recent.insert(0, color);
    recent.truncate(max);
}
