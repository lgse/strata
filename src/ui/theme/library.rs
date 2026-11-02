// SPDX-License-Identifier: MIT

//! The themes Strata lists: the bundled catalog and the user's theme files,
//! including the dotfile links that point at them.

use std::{
    fs,
    path::{Path, PathBuf},
};

use gtk::{gdk, gio, prelude::*};
use serde::Deserialize;

use super::{Theme, ThemeTokens, canonical_color, default_danger, omarchy};

const THEME_CATALOG: &str = include_str!("../../../data/themes/catalog.toml");

#[derive(Deserialize)]
struct ThemeCatalog {
    themes: Vec<CatalogTheme>,
}

#[derive(Deserialize)]
struct CatalogTheme {
    id: String,
    #[serde(flatten)]
    tokens: ThemeTokens,
}

pub(super) fn builtins() -> Vec<Theme> {
    let mut themes: Vec<_> = toml::from_str::<ThemeCatalog>(THEME_CATALOG)
        .map(|catalog| {
            catalog
                .themes
                .into_iter()
                .map(|theme| Theme {
                    id: theme.id,
                    tokens: theme.tokens,
                    custom: false,
                })
                .collect()
        })
        .unwrap_or_default();
    for theme in &mut themes {
        // Dim text carries the same contrast floors as body text, and the lift
        // stops at the first passing step, so it stays dimmer than text.
        let tokens = &mut theme.tokens;
        let dim = omarchy::readable(&tokens.dim_text, &[&tokens.background], &tokens.text, 4.5);
        tokens.dim_text = omarchy::readable(&dim, &[&tokens.surface], &tokens.text, 3.0);
    }
    themes.sort_by_key(|theme| theme.tokens.name.to_lowercase());
    themes
}

pub(super) fn merge_builtin_and_custom_themes(
    mut builtins: Vec<Theme>,
    custom: Vec<Theme>,
) -> Vec<Theme> {
    builtins.retain(|builtin| !custom.iter().any(|theme| theme.id == builtin.id));
    builtins.extend(custom);
    builtins
}

pub(super) fn azure_tokens() -> ThemeTokens {
    builtins()
        .into_iter()
        .find(|theme| theme.id == "azure-glow")
        .map(|theme| theme.tokens)
        .unwrap_or_else(|| ThemeTokens {
            name: "Azure Glow".to_owned(),
            background: "#0c1a2b".to_owned(),
            surface: "#122438".to_owned(),
            text: "#c9deed".to_owned(),
            accent: "#4fd6ff".to_owned(),
            danger: default_danger(),
            muted: "#1e3a52".to_owned(),
            highlight: "#244d68".to_owned(),
            border: "#315b75".to_owned(),
            dim_text: "#6f8da3".to_owned(),
            syntax_keyword: None,
            syntax_string: None,
            syntax_constant: None,
            syntax_type: None,
            syntax_preprocessor: None,
        })
}

pub(super) fn load_custom_themes() -> Vec<Theme> {
    let Ok(entries) = fs::read_dir(themes_directory()) else {
        return Vec::new();
    };
    let mut themes: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "toml")
        })
        .filter_map(|entry| {
            let id = entry.path().file_stem()?.to_string_lossy().into_owned();
            let source = fs::read_to_string(entry.path()).ok()?;
            let mut tokens: ThemeTokens = toml::from_str(&source).ok()?;
            validate_tokens(&tokens).ok()?;
            // Canonical in memory only; the user's file stays as written.
            for color in tokens.color_fields_mut() {
                *color = canonical_color(color)?;
            }
            Some(Theme {
                id,
                tokens,
                custom: true,
            })
        })
        .collect();
    themes.sort_by(|left, right| left.tokens.name.cmp(&right.tokens.name));
    themes
}

pub(super) fn validate_tokens(tokens: &ThemeTokens) -> Result<(), &'static str> {
    if tokens.name.trim().is_empty() {
        return Err("Enter a theme name");
    }
    if tokens
        .color_fields()
        .any(|color| gdk::RGBA::parse(color).is_err())
    {
        return Err("Every color must be a valid CSS color");
    }
    Ok(())
}

pub(super) fn is_custom_theme_event(file: &gio::File) -> bool {
    file.path().is_some_and(|path| {
        path.parent() == Some(themes_directory().as_path())
            && path
                .extension()
                .is_some_and(|extension| extension == "toml")
    })
}

pub(super) fn linked_theme_targets(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "toml")
                && entry
                    .file_type()
                    .is_ok_and(|file_type| file_type.is_symlink())
        })
        .filter_map(|entry| resolve_theme_link(&entry.path()))
        .collect()
}

/// A missing target still resolves, so its directory is watched until the file appears.
fn resolve_theme_link(link: &Path) -> Option<PathBuf> {
    fs::canonicalize(link).ok().or_else(|| {
        let target = link.parent()?.join(fs::read_link(link).ok()?);
        Some(
            fs::canonicalize(target.parent()?)
                .ok()?
                .join(target.file_name()?),
        )
    })
}

pub(super) fn slugify(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .fold(String::new(), |mut slug, character| {
            if character.is_ascii_alphanumeric() {
                slug.push(character);
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
            slug
        })
        .trim_end_matches('-')
        .to_owned()
}

const FALLBACK_CUSTOM_THEME_ID: &str = "custom-theme";

/// File id for a new custom theme: the ASCII slug of `name`, or a generic id when the
/// name has no ASCII letters or digits, suffixed until `taken` no longer matches.
pub(super) fn custom_theme_id(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let slug = slugify(name);
    let base = if slug.is_empty() {
        FALLBACK_CUSTOM_THEME_ID
    } else {
        slug.as_str()
    };
    let mut id = base.to_owned();
    let mut suffix = 2;
    while taken(&id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    id
}

pub(super) fn themes_directory() -> PathBuf {
    super::super::preferences::config_directory().join("themes")
}
