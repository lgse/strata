// SPDX-License-Identifier: MIT

//! What to look up: names with their fallbacks, or a file, at a size and scale.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// Where an icon comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IconSource {
    /// Icon names in order of preference, looked up in the theme.
    Names(Vec<String>),
    /// An image file, such as a `.desktop` entry's absolute `Icon=` path.
    File(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct IconRequest {
    pub source: IconSource,
    pub size: u32,
    pub scale: f32,
}

impl IconRequest {
    pub fn named<N: Into<String>>(
        names: impl IntoIterator<Item = N>,
        size: u32,
        scale: f32,
    ) -> Self {
        let names = names.into_iter().map(Into::into).collect();
        Self {
            source: IconSource::Names(names),
            size,
            scale,
        }
    }

    pub fn file(path: impl Into<PathBuf>, size: u32, scale: f32) -> Self {
        Self {
            source: IconSource::File(path.into()),
            size,
            scale,
        }
    }

    pub fn from_desktop_icon(value: &str, size: u32, scale: f32) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        if Path::new(value).is_absolute() {
            return Some(Self::file(value, size, scale));
        }
        let name = match value.rfind('.') {
            Some(dot) if matches!(&value[dot..], ".png" | ".xpm" | ".svg") => &value[..dot],
            _ => value,
        };
        Some(Self::named(themed_names(&[name], false), size, scale))
    }

    pub fn pixels(&self) -> u32 {
        let px = (self.size as f32 * self.scale).ceil();
        if px.is_finite() {
            px.clamp(1.0, 1024.0) as u32
        } else {
            1
        }
    }

    pub fn theme_scale(&self) -> u32 {
        let scale = self.scale.ceil();
        if scale.is_finite() {
            scale.clamp(1.0, 8.0) as u32
        } else {
            1
        }
    }
}

pub fn themed_names(names: &[&str], dash_fallbacks: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for &given in names {
        let base = given.strip_suffix("-symbolic");
        let symbolic = base.is_some();
        let name = base.unwrap_or(given);
        // GLib checks for the plain name even when it adds the symbolic one.
        if out.iter().any(|n| n == name) {
            continue;
        }
        out.push(given.to_owned());
        if !dash_fallbacks {
            continue;
        }
        let mut last = name;
        while let Some(dash) = last.rfind('-') {
            last = &last[..dash];
            let fallback = if symbolic {
                format!("{last}-symbolic")
            } else {
                last.to_owned()
            };
            if out.contains(&fallback) {
                break;
            }
            out.push(fallback);
        }
    }
    let variants: Vec<String> = out
        .iter()
        .map(|n| match n.strip_suffix("-symbolic") {
            Some(plain) => plain.to_owned(),
            None => format!("{n}-symbolic"),
        })
        .filter(|v| !out.contains(v))
        .collect();
    let mut seen = Vec::new();
    for v in variants {
        if !seen.contains(&v) {
            seen.push(v);
        }
    }
    out.extend(seen);
    out
}
