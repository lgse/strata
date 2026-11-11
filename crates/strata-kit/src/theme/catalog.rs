//! Strata's bundled themes: `themes/catalog.toml`, compiled in and read once.
//!
//! As Strata's `builtins()` (`src/ui/theme/library.rs` at lgse/strata 35ee677ab): each
//! entry's dim text is lifted to the contrast floors body text carries (4.5:1 on the
//! background, then 3:1 on the surface), stopping at the first step that passes so it stays
//! dimmer than text, and the list is sorted by lowercase name.

use std::sync::OnceLock;

use toml::de::DeValue;

use super::color::readable;
use super::colors::{Location, Source, Theme, parse_document, read_table};

#[cfg(test)]
mod tests;

const CATALOG: &str = include_str!("../../themes/catalog.toml");

/// The theme a fresh start uses: Tokyo Night.
pub const DEFAULT_ID: &str = "tokyo-night";

/// The theme a saved id that matches nothing falls back to: Azure Glow.
pub const UNKNOWN_ID_FALLBACK: &str = "azure-glow";

/// Every bundled theme, sorted by lowercase name.
pub fn bundled() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| parse(CATALOG).unwrap_or_else(|e| panic!("themes/catalog.toml: {e}")))
}

/// The bundled theme with id `id`.
pub fn bundled_theme(id: &str) -> Option<&'static Theme> {
    bundled().iter().find(|t| t.id == id)
}

/// Reads a catalogue: `[[themes]]` entries, each an `id` plus a theme's keys.
pub(crate) fn parse(src: &str) -> Result<Vec<Theme>, String> {
    let doc = parse_document(src).map_err(|e| e.to_string())?;
    let entries = doc
        .get_ref()
        .get("themes")
        .and_then(|v| v.get_ref().as_array())
        .ok_or("no [[themes]] array")?;
    let mut themes = Vec::with_capacity(entries.len());
    for entry in entries {
        let line = Location::of(src, entry.span().start).line;
        let DeValue::Table(table) = entry.get_ref() else {
            return Err(format!("line {line}: a theme must be a table"));
        };
        let Some(DeValue::String(id)) = table.get("id").map(|v| v.get_ref()) else {
            return Err(format!("line {line}: a theme needs a string `id`"));
        };
        let (name, mut colors) = read_table(src, table).map_err(|e| e.to_string())?;
        let dim = readable(colors.dim_text, &[colors.background], colors.text, 4.5);
        colors.dim_text = readable(dim, &[colors.surface], colors.text, 3.0);
        themes.push(Theme {
            id: id.to_string(),
            name,
            colors,
            source: Source::Bundled,
        });
    }
    themes.sort_by_key(|t| t.name.to_lowercase());
    Ok(themes)
}
