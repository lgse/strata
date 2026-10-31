// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::REPOSITORY;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    mtime: i64,
    trees: BTreeMap<String, Value>,
    pub(crate) preferences: OrderedMap,
    pub(crate) gtk_settings: String,
    pub(crate) portal_offer_marker: String,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) layouts: BTreeMap<String, Layout>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Layout {
    pub(crate) layouts: String,
    pub(crate) group: u8,
    pub(crate) caps_lock: bool,
    pub(crate) variants: String,
}

/// Keeps document order, so `settings.toml` lists preferences as the manifest does.
#[derive(Clone, Debug, Default)]
pub(crate) struct OrderedMap(pub(crate) Vec<(String, Value)>);

impl OrderedMap {
    pub(crate) fn update(&mut self, key: &str, value: Value) {
        match self.0.iter_mut().find(|(existing, _)| existing == key) {
            Some(entry) => entry.1 = value,
            None => self.0.push((key.to_owned(), value)),
        }
    }
}

impl<'de> Deserialize<'de> for OrderedMap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = OrderedMap;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<OrderedMap, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Value>()? {
                    entries.push(entry);
                }
                Ok(OrderedMap(entries))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

pub(crate) fn manifest() -> Manifest {
    let path = Path::new(REPOSITORY).join("tests/fixtures/harness.json");
    let text = fs::read_to_string(&path).expect("the harness manifest should be readable");
    serde_json::from_str(&text).expect("the harness manifest should parse")
}

pub(crate) fn pin_times(path: &Path, mtime: SystemTime) {
    if path.is_dir() {
        let mut children: Vec<PathBuf> = fs::read_dir(path)
            .expect("the fixture directory should be listable")
            .map(|entry| entry.expect("the fixture entry should be readable").path())
            .collect();
        children.sort();
        for child in children {
            pin_times(&child, mtime);
        }
    }
    File::open(path)
        .and_then(|file| {
            file.set_times(fs::FileTimes::new().set_accessed(mtime).set_modified(mtime))
        })
        .expect("the fixture time should be pinned");
}

pub(crate) fn listing(root: &Path) -> String {
    fn walk(root: &Path, directory: &Path, lines: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for path in paths {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if path.is_symlink() {
                let target = fs::read_link(&path).unwrap_or_default();
                lines.push(format!("{relative} -> {}", target.display()));
            } else if path.is_dir() {
                lines.push(format!("{relative}/"));
                walk(root, &path, lines);
            } else {
                let size = fs::metadata(&path).map_or(0, |metadata| metadata.len());
                lines.push(format!("{relative} ({size} bytes)"));
            }
        }
    }
    let mut lines = Vec::new();
    walk(root, root, &mut lines);
    lines.join("\n")
}

pub(crate) static FIXED_FIXTURES: Mutex<()> = Mutex::new(());

pub(crate) fn fixed_fixture_name(root: &str) -> Option<&str> {
    let name = root.strip_prefix("/tmp/")?;
    (!name.is_empty() && !name.contains('/') && name != "." && name != "..").then_some(name)
}

pub(crate) fn gtk_settings(
    base: &str,
    changes: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut lines: Vec<String> = base.lines().map(str::to_owned).collect();
    for (key, value) in changes {
        let line = lines
            .iter_mut()
            .find(|line| line.split_once('=').is_some_and(|(name, _)| name == key))
            .ok_or_else(|| format!("the manifest's GTK settings have no {key}"))?;
        *line = format!("{key}={value}");
    }
    let mut text = lines.join("\n");
    if base.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

pub(crate) fn expand_tree(tree: &Value, manifest: &Manifest) -> Result<Value, String> {
    match tree {
        Value::String(name) => manifest
            .trees
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no fixture tree {name:?} in the manifest")),
        Value::Object(_) => expand_directory(tree),
        other => Err(format!(
            "tree is a manifest tree name or a tree, not {other}"
        )),
    }
}

pub(crate) fn expand_directory(directory: &Value) -> Result<Value, String> {
    let Value::Object(entries) = directory else {
        return Err("a fixture directory must be an object".to_owned());
    };
    let mut expanded = Map::new();
    for (name, value) in entries {
        let child = match value {
            Value::Object(_) => expand_directory(value)?,
            Value::String(_) => value.clone(),
            Value::Array(parts) => Value::String(file_parts(name, parts)?),
            _ => {
                return Err(format!(
                    "fixture {name} must be a file's text or a directory"
                ));
            }
        };
        for (expanded_name, number) in numbered(name)? {
            let contents = match (&child, number) {
                (Value::String(text), Some(number)) => {
                    Value::String(text.replace("{}", &number.to_string()))
                }
                (other, _) => other.clone(),
            };
            expanded.insert(expanded_name, contents);
        }
    }
    Ok(Value::Object(expanded))
}

fn file_parts(name: &str, parts: &[Value]) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Repeat {
        repeat: String,
        from: u32,
        to: u32,
    }
    let mut text = String::new();
    for part in parts {
        match part {
            Value::String(part) => text.push_str(part),
            other => {
                let repeat: Repeat = serde_json::from_value(other.clone()).map_err(|error| {
                    format!("fixture {name}: a part is text or a repeat: {error}")
                })?;
                for number in repeat.from..=repeat.to {
                    text.push_str(&repeat.repeat.replace("{}", &number.to_string()));
                }
            }
        }
    }
    Ok(text)
}

pub(crate) fn expand_name(name: &str) -> Result<Vec<String>, String> {
    Ok(numbered(name)?.into_iter().map(|(name, _)| name).collect())
}

pub(crate) fn numbered(name: &str) -> Result<Vec<(String, Option<u32>)>, String> {
    let range = name.split_once('{').and_then(|(prefix, rest)| {
        let (range, suffix) = rest.split_once('}')?;
        let (first, last) = range.split_once("..")?;
        Some((prefix, first, last.parse::<u32>().ok()?, suffix))
    });
    let Some((prefix, first, last, suffix)) = range else {
        return Ok(vec![(name.to_owned(), None)]);
    };
    let start: u32 = first
        .parse()
        .map_err(|_| format!("fixture {name}: the range must be numeric"))?;
    Ok((start..=last)
        .map(|number| {
            (
                format!("{prefix}{number:0width$}{suffix}", width = first.len()),
                Some(number),
            )
        })
        .collect())
}

pub(crate) fn fixture_time(manifest: &Manifest) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(manifest.mtime.unsigned_abs())
}
