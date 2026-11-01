// SPDX-License-Identifier: MIT

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    model::Location,
    storage::writer::{self, Policy},
};

const SESSION_VERSION: u32 = 1;
const SESSION_FILE_NAME: &str = "tabs.toml";
const MAX_RESTORED_TABS: usize = 32;
const KIND_PATH: &str = "path";
const KIND_URI: &str = "uri";

const RESTORABLE_URI_SCHEMES: [&str; 9] = [
    "smb", "sftp", "ftp", "ftps", "dav", "davs", "trash", "network", "recent",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StoredTab {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StoredSession {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    active: usize,
    #[serde(default)]
    tabs: Vec<StoredTab>,
}

pub(crate) struct RestoredSession {
    pub tabs: Vec<Location>,
    pub active: usize,
}

pub(crate) fn session_path() -> PathBuf {
    crate::storage::config_directory().join(SESSION_FILE_NAME)
}

pub(crate) fn save(tabs: &[Location], active: usize) {
    match session_contents(tabs, active) {
        Ok(contents) => drop(writer::save(
            session_path(),
            Some(contents.into_bytes()),
            Policy::State,
            Some("unable to save open tabs"),
        )),
        Err(error) => tracing::warn!(%error, "unable to save open tabs"),
    }
}

pub(crate) fn remove() {
    drop(writer::save(
        session_path(),
        None,
        Policy::State,
        Some("unable to clear saved tabs"),
    ));
}

pub(crate) fn load_restorable() -> Option<RestoredSession> {
    load_from(&session_path())
}

fn session_contents(tabs: &[Location], active: usize) -> io::Result<String> {
    let mut stored = Vec::new();
    let mut stored_active = 0;
    for (index, location) in tabs.iter().enumerate() {
        let entry = if let Some(path) = location.native_path() {
            path.to_str()
                .filter(|_| path.is_absolute())
                .map(|value| StoredTab::from(KIND_PATH, value))
        } else {
            location
                .uri_value()
                .and_then(|value| restorable_uri(value).map(|_| StoredTab::from(KIND_URI, value)))
        };
        if let Some(entry) = entry {
            if index <= active {
                stored_active = stored.len();
            }
            stored.push(entry);
        }
    }
    let session = StoredSession {
        version: SESSION_VERSION,
        active: stored_active,
        tabs: stored,
    };
    toml::to_string(&session).map_err(io::Error::other)
}

pub(crate) fn load_from(path: &Path) -> Option<RestoredSession> {
    let contents = fs::read_to_string(path).ok()?;
    let stored: StoredSession = toml::from_str(&contents).ok()?;
    if stored.version != SESSION_VERSION {
        return None;
    }
    let mut tabs = Vec::new();
    let mut active = 0;
    for (index, tab) in stored.tabs.iter().enumerate() {
        if let Some(location) = restorable_location(&tab.kind, &tab.value) {
            if index <= stored.active {
                active = tabs.len();
            }
            tabs.push(location);
            if tabs.len() == MAX_RESTORED_TABS {
                break;
            }
        }
    }
    if tabs.is_empty() {
        return None;
    }
    Some(RestoredSession { active, tabs })
}

impl StoredTab {
    fn from(kind: &str, value: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            value: value.to_owned(),
        }
    }
}

fn restorable_location(kind: &str, value: &str) -> Option<Location> {
    match kind {
        KIND_PATH => {
            let path = PathBuf::from(value);
            path.is_absolute().then(|| Location::local(path))
        }
        KIND_URI => restorable_uri(value),
        _ => None,
    }
}

fn restorable_uri(value: &str) -> Option<Location> {
    let uri = gio::glib::Uri::parse(
        value,
        gio::glib::UriFlags::HAS_PASSWORD | gio::glib::UriFlags::HAS_AUTH_PARAMS,
    )
    .ok()?;
    let scheme = uri.scheme().to_ascii_lowercase();
    if !RESTORABLE_URI_SCHEMES.contains(&scheme.as_str())
        || crate::model::uri_contains_credentials(&uri)
    {
        return None;
    }
    match scheme.as_str() {
        "trash" if value != "trash:///" => return None,
        "recent" if value != "recent:///" => return None,
        _ => {}
    }
    Some(Location::uri(value))
}

#[cfg(test)]
mod tests;
