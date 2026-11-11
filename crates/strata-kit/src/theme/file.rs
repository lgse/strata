//! Custom theme files and the library they join.
//!
//! A custom theme is a TOML file in `~/.config/strata/themes/`, keyed by its file stem.
//! Strata reads, watches and saves those files; this takes a file's contents as data and
//! returns the theme, or the first problem with its line. As Strata's `load_custom_themes`
//! and `merge_builtin_and_custom_themes` (`src/ui/theme/library.rs` at lgse/strata 35ee677ab):
//!
//! - colours load canonical in memory; the file stays as written;
//! - a custom file keeps its dim text as written, even below the floors bundled themes meet;
//! - `danger` defaults to `#e5484d`, and each `syntax_*` key is optional;
//! - a custom id that matches a bundled theme replaces it.
//!
//! Strata skips a file that doesn't load. Reporting where it failed is new here.

use super::catalog::{DEFAULT_ID, UNKNOWN_ID_FALLBACK, bundled};
use super::colors::{Source, Theme, ThemeError, parse_document, read_table};

#[cfg(test)]
mod tests;

/// The custom theme in `src`, the contents of the file `<id>.toml`.
pub fn parse_custom(id: &str, src: &str) -> Result<Theme, ThemeError> {
    let doc = parse_document(src)?;
    let (name, colors) = read_table(src, doc.get_ref())?;
    Ok(Theme {
        id: id.to_owned(),
        name,
        colors,
        source: Source::Custom,
    })
}

/// Every theme Strata lists: the bundled ones, minus any a custom theme replaces, then the
/// custom ones sorted by name.
#[derive(Clone, Debug)]
pub struct Library {
    themes: Vec<Theme>,
}

impl Library {
    pub fn new(mut custom: Vec<Theme>) -> Library {
        custom.sort_by(|a, b| a.name.cmp(&b.name));
        let mut themes: Vec<Theme> = bundled()
            .iter()
            .filter(|b| !custom.iter().any(|c| c.id == b.id))
            .cloned()
            .collect();
        themes.extend(custom);
        Library { themes }
    }

    pub fn themes(&self) -> &[Theme] {
        &self.themes
    }

    pub fn get(&self, id: &str) -> Option<&Theme> {
        self.themes.iter().find(|t| t.id == id)
    }

    /// The theme a saved selection names: Tokyo Night when nothing is saved, Azure Glow when
    /// the saved id matches no theme.
    pub fn resolve(&self, saved: Option<&str>) -> &Theme {
        let id = saved.unwrap_or(DEFAULT_ID);
        self.get(id)
            .or_else(|| self.get(UNKNOWN_ID_FALLBACK))
            .or_else(|| self.themes.first())
            .unwrap_or_else(|| panic!("the catalogue holds {UNKNOWN_ID_FALLBACK}"))
    }
}

impl Default for Library {
    /// The bundled themes alone.
    fn default() -> Library {
        Library::new(Vec::new())
    }
}
