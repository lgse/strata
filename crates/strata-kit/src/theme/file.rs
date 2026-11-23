// SPDX-License-Identifier: MIT

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
