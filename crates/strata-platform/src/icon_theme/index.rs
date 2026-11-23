// SPDX-License-Identifier: MIT

//! `index.theme`: the theme's inherited themes and its directories' size parameters.

use std::collections::HashMap;

#[cfg(test)]
mod tests;

/// How a directory's icons match a requested size (the spec's `Type` key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct DirParams {
    pub kind: DirKind,
    pub size: i32,
    pub min: i32,
    pub max: i32,
    pub threshold: i32,
    pub scale: i32,
}

/// One entry of `Directories` or `ScaledDirectories`, relative to the theme's directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DirSpec {
    pub path: String,
    pub params: DirParams,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ThemeIndex {
    pub inherits: Vec<String>,
    pub dirs: Vec<DirSpec>,
}

pub(super) fn parse(text: &str) -> Option<ThemeIndex> {
    let parsed = key_file(text);
    let mut groups: HashMap<&str, &[(String, String)]> = HashMap::new();
    for (name, keys) in &parsed {
        groups.entry(name.as_str()).or_insert(keys.as_slice());
    }
    let group = |name: &str| groups.get(name).copied();
    let head = group("Icon Theme")?;
    let listed = list(value(head, "Directories")?);
    let scaled = value(head, "ScaledDirectories")
        .map(list)
        .unwrap_or_default();
    let mut dirs: Vec<DirSpec> = Vec::new();
    for path in listed.into_iter().chain(scaled) {
        if !inside_theme(&path) || dirs.iter().any(|d| d.path == path) {
            continue;
        }
        let Some(params) = group(&path).and_then(dir_params) else {
            continue;
        };
        dirs.push(DirSpec { path, params });
    }
    let inherits = value(head, "Inherits").map(list).unwrap_or_default();
    Some(ThemeIndex { inherits, dirs })
}

fn inside_theme(path: &str) -> bool {
    use std::path::{Component, Path};
    Path::new(path)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn dir_params(keys: &[(String, String)]) -> Option<DirParams> {
    let size = int(value(keys, "Size")?)?;
    let kind = match value(keys, "Type") {
        Some("Fixed") => DirKind::Fixed,
        Some("Scalable") => DirKind::Scalable,
        _ => DirKind::Threshold,
    };
    let or = |key: &str, default: i32| value(keys, key).and_then(int).unwrap_or(default);
    Some(DirParams {
        kind,
        size,
        min: or("MinSize", size),
        max: or("MaxSize", size),
        threshold: or("Threshold", 2),
        scale: or("Scale", 1).max(1),
    })
}

type Group = (String, Vec<(String, String)>);

fn key_file(text: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            groups.push((name.to_owned(), Vec::new()));
            continue;
        }
        let (Some((_, keys)), Some((key, value))) = (groups.last_mut(), line.split_once('='))
        else {
            continue;
        };
        let key = key.trim();
        if !key.contains('[') {
            keys.push((key.to_owned(), value.trim().to_owned()));
        }
    }
    groups
}

/// The last value of `key`, as `GKeyFile` keeps the last of a repeated key.
fn value<'a>(keys: &'a [(String, String)], key: &str) -> Option<&'a str> {
    keys.iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn int(value: &str) -> Option<i32> {
    value.trim().parse().ok()
}

pub(super) fn derived_hicolor(dirs: impl IntoIterator<Item = String>) -> ThemeIndex {
    let mut out: Vec<DirSpec> = Vec::new();
    for path in dirs {
        let Some(class) = path.split('/').next() else {
            continue;
        };
        let threshold = |size, scale| DirParams {
            kind: DirKind::Threshold,
            size,
            min: size,
            max: size,
            threshold: 2,
            scale,
        };
        let scalable = |size, min, max| DirParams {
            kind: DirKind::Scalable,
            size,
            min,
            max,
            threshold: 2,
            scale: 1,
        };
        let params = match class {
            "scalable" => Some(scalable(128, 1, 256)),
            "symbolic" => Some(scalable(16, 8, 512)),
            _ => {
                let (dims, scale) = class.split_once('@').unwrap_or((class, "1"));
                let scale = scale.strip_suffix('x').unwrap_or(scale);
                let size = dims.split_once('x').and_then(|(w, _)| int(w));
                size.zip(int(scale))
                    .map(|(size, scale)| threshold(size, scale.max(1)))
            }
        };
        if let Some(params) = params
            && !out.iter().any(|d| d.path == path)
        {
            out.push(DirSpec { path, params });
        }
    }
    ThemeIndex {
        inherits: Vec::new(),
        dirs: out,
    }
}
