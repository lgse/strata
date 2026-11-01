// SPDX-License-Identifier: MIT

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{
    XDG_DIRECTORIES,
    scenario::{Header, Variant, view_name},
};

pub(crate) struct Run {
    pub(crate) mode: String,
    pub(crate) variant: Variant,
    pub(crate) label: String,
}

pub(crate) fn runs(header: &Header) -> Vec<Run> {
    let variants = if header.variants.is_empty() {
        vec![Variant::default()]
    } else {
        header.variants.clone()
    };
    let named = !header.variants.is_empty();
    variants
        .into_iter()
        .flat_map(|variant| {
            let modes = variant
                .modes
                .clone()
                .unwrap_or_else(|| header.modes.clone());
            modes.into_iter().map(move |mode| Run {
                label: if named {
                    format!("{mode}-{}", variant.id)
                } else {
                    mode.clone()
                },
                mode,
                variant: variant.clone(),
            })
        })
        .collect()
}

pub(crate) struct Places<'a> {
    pub(crate) fixture: &'a Path,
    pub(crate) scratch_home: &'a Path,
}

impl Places<'_> {
    fn xdg(&self, variable: &str) -> PathBuf {
        let (_, directory) = XDG_DIRECTORIES
            .iter()
            .find(|(name, _)| *name == variable)
            .expect("every built-in names an XDG directory");
        self.scratch_home.join(directory)
    }
}

pub(crate) fn placeholders() -> Places<'static> {
    Places {
        fixture: Path::new("/tmp/strata-qa-fixture-placeholder"),
        scratch_home: Path::new("/tmp/strata-qa-placeholder/home"),
    }
}

pub(crate) fn variables(run: &Run, places: &Places) -> Vec<(String, String)> {
    let mut variables: Vec<(String, String)> = run.variant.vars.clone().into_iter().collect();
    let root = places
        .fixture
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    variables.push(("root".to_owned(), root));
    variables.push(("fixture".to_owned(), places.fixture.display().to_string()));
    variables.push(("home".to_owned(), places.xdg("HOME").display().to_string()));
    variables.push((
        "data".to_owned(),
        places.xdg("XDG_DATA_HOME").display().to_string(),
    ));
    variables.push((
        "config".to_owned(),
        places.xdg("XDG_CONFIG_HOME").display().to_string(),
    ));
    variables.push((
        "cache".to_owned(),
        places.xdg("XDG_CACHE_HOME").display().to_string(),
    ));
    variables.push(("python".to_owned(), PYTHON.to_owned()));
    variables.push((
        "fixture_quoted".to_owned(),
        quote_path(places.fixture.as_os_str().as_encoded_bytes()),
    ));
    variables.push((
        "view".to_owned(),
        view_name(&run.mode).unwrap_or_default().to_owned(),
    ));
    variables.push(("mode".to_owned(), run.mode.clone()));
    variables.push((
        "environment".to_owned(),
        places.scratch_home.display().to_string(),
    ));
    variables.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    variables
}

/// Absolute, as a file provider's manifest command must be.
pub(crate) const PYTHON: &str = "/usr/bin/python3";

pub(crate) fn quote_path(bytes: &[u8]) -> String {
    let mut quoted = String::with_capacity(bytes.len());
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || b"_.-~/".contains(byte) {
            quoted.push(char::from(*byte));
        } else {
            let _ = write!(quoted, "%{byte:02X}");
        }
    }
    quoted
}

pub(crate) fn run_name(scenario: &str, run: &Run) -> String {
    format!("{scenario}/{}", run.label)
}

pub(crate) fn substitute(value: &Value, variables: &[(String, String)]) -> Value {
    match value {
        Value::String(text) => Value::String(substitute_text(text, variables)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute(item, variables))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, item)| (key.clone(), substitute(item, variables)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `$name` is a whole token. `$$` stays for [`unescape`], so a later pass cannot read
/// `$$E` as `$E`.
pub(crate) fn substitute_text(text: &str, variables: &[(String, String)]) -> String {
    let identifier = |character: char| character.is_ascii_alphanumeric() || character == '_';
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(escaped) = after.strip_prefix('$') {
            output.push_str("$$");
            rest = escaped;
            continue;
        }
        let length = after.find(|c: char| !identifier(c)).unwrap_or(after.len());
        let token = &after[..length];
        match variables.iter().find(|(name, _)| name == token) {
            Some((_, replacement)) if !token.is_empty() => output.push_str(replacement),
            _ => {
                output.push('$');
                output.push_str(token);
            }
        }
        rest = &after[length..];
    }
    output.push_str(rest);
    output
}

pub(crate) fn unescape(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(text.replace("$$", "$")),
        Value::Array(items) => Value::Array(items.iter().map(unescape).collect()),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, item)| (key.clone(), unescape(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}
