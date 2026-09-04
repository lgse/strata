// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cmp::Ordering, ffi::OsString, path::PathBuf};

use gio::prelude::*;

/// A browsable destination. Native paths remain byte-safe and URI locations remain explicit.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum LocationKind {
    Native(PathBuf),
    Uri(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Location {
    kind: LocationKind,
}

pub(crate) fn uri_contains_credentials(uri: &gio::glib::Uri) -> bool {
    uri.password().is_some()
        || uri.auth_params().is_some()
        || uri.user().is_some_and(|user| user.contains([':', ';']))
}

impl Location {
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: LocationKind::Native(path.into()),
        }
    }

    pub fn uri(uri: impl Into<String>) -> Self {
        Self {
            kind: LocationKind::Uri(uri.into()),
        }
    }

    pub fn native_path(&self) -> Option<&std::path::Path> {
        match &self.kind {
            LocationKind::Native(path) => Some(path),
            LocationKind::Uri(_) => None,
        }
    }

    pub fn uri_value(&self) -> Option<&str> {
        match &self.kind {
            LocationKind::Native(_) => None,
            LocationKind::Uri(uri) => Some(uri),
        }
    }

    pub fn parent(&self) -> Option<Self> {
        match &self.kind {
            LocationKind::Native(path) => {
                let parent = path.parent()?;
                (parent != path).then(|| Self::local(parent))
            }
            LocationKind::Uri(uri) if uri == "trash:///" || uri == "network:///" => None,
            LocationKind::Uri(uri) => {
                let file = gio::File::for_uri(uri);
                let parent = file.parent()?;
                let parent_uri = parent.uri();
                let canonical = if parent_uri.ends_with("///") {
                    parent_uri.to_string()
                } else {
                    parent_uri.trim_end_matches('/').to_owned()
                };
                let location = Self::uri(canonical);
                (&location != self).then_some(location)
            }
        }
    }

    pub fn is_absolute_native(&self) -> bool {
        self.native_path().is_some_and(std::path::Path::is_absolute)
    }

    /// Byte-safe file name for native paths, and the decoded final segment for URIs.
    pub fn file_name(&self) -> Option<OsString> {
        match &self.kind {
            LocationKind::Native(path) => path.file_name().map(OsString::from),
            LocationKind::Uri(uri) => gio::File::for_uri(uri)
                .basename()?
                .file_name()
                .map(OsString::from),
        }
    }

    /// Resolves a direct child by name, rejecting names that would escape `self`.
    pub fn child(&self, name: &std::ffi::OsStr) -> Option<Self> {
        if name.is_empty() || matches!(name.as_encoded_bytes(), b"." | b"..") {
            return None;
        }
        if name.as_encoded_bytes().contains(&b'/') {
            return None;
        }
        match &self.kind {
            LocationKind::Native(path) => Some(Self::local(path.join(name))),
            LocationKind::Uri(uri) => {
                let child = gio::File::for_uri(uri).child(name);
                Some(Self::uri(child.uri().to_string()))
            }
        }
    }

    /// Where an item lands when transferred into `destination` without renaming.
    pub fn transfer_target(&self, destination: &Self) -> Option<Self> {
        destination.child(&self.file_name()?)
    }

    pub fn rebase(&self, from: &Self, to: &Self) -> Option<Self> {
        let suffix = self.native_path()?.strip_prefix(from.native_path()?).ok()?;
        Some(Self::local(to.native_path()?.join(suffix)))
    }

    pub fn is_within(&self, other: &Self) -> bool {
        self.native_path()
            .zip(other.native_path())
            .is_some_and(|(path, parent)| path.starts_with(parent))
    }

    pub fn compare(&self, other: &Self) -> Ordering {
        match (&self.kind, &other.kind) {
            (LocationKind::Native(left), LocationKind::Native(right)) => left.cmp(right),
            (LocationKind::Uri(left), LocationKind::Uri(right)) => left.cmp(right),
            (LocationKind::Native(_), LocationKind::Uri(_)) => Ordering::Less,
            (LocationKind::Uri(_), LocationKind::Native(_)) => Ordering::Greater,
        }
    }

    pub fn backend_name(&self) -> String {
        match &self.kind {
            LocationKind::Native(_) => "native".into(),
            LocationKind::Uri(uri) => gio::glib::Uri::parse_scheme(uri)
                .map(|scheme| scheme.to_string())
                .unwrap_or_else(|| "uri".into()),
        }
    }

    /// Returns a debug-only location with URI user-info, query, and fragment removed.
    pub fn diagnostic_path(&self) -> String {
        match &self.kind {
            LocationKind::Native(path) => path.to_string_lossy().into_owned(),
            LocationKind::Uri(uri) => gio::glib::Uri::parse(
                uri,
                gio::glib::UriFlags::HAS_PASSWORD | gio::glib::UriFlags::HAS_AUTH_PARAMS,
            )
            .map(|uri| {
                uri.to_string_partial(
                    gio::glib::UriHideFlags::USERINFO
                        | gio::glib::UriHideFlags::QUERY
                        | gio::glib::UriHideFlags::FRAGMENT,
                )
                .to_string()
            })
            .unwrap_or_else(|_| "<invalid-uri>".into()),
        }
    }

    /// Returns a UTF-8-safe representation without changing the native path.
    pub fn display_path(&self) -> String {
        match &self.kind {
            LocationKind::Native(path) => path.to_string_lossy().into_owned(),
            LocationKind::Uri(uri) => gio::glib::Uri::parse(
                uri,
                gio::glib::UriFlags::HAS_PASSWORD | gio::glib::UriFlags::HAS_AUTH_PARAMS,
            )
            .map(|uri| {
                let hidden = if uri_contains_credentials(&uri) {
                    gio::glib::UriHideFlags::USERINFO
                } else {
                    gio::glib::UriHideFlags::empty()
                };
                uri.to_string_partial(hidden).to_string()
            })
            .unwrap_or_else(|_| "<invalid-uri>".into()),
        }
    }

    pub fn display_name(&self) -> String {
        match &self.kind {
            LocationKind::Native(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            LocationKind::Uri(uri) if uri == "trash:///" => "Trash".into(),
            LocationKind::Uri(uri) => uri
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(uri)
                .into(),
        }
    }

    pub fn breadcrumbs(&self) -> Vec<Self> {
        if let Some(path) = self.native_path() {
            let mut locations: Vec<_> = path.ancestors().map(Self::local).collect();
            locations.reverse();
            return locations;
        }
        let mut locations = vec![self.clone()];
        while let Some(parent) = locations.last().and_then(Self::parent) {
            if locations.contains(&parent) {
                break;
            }
            locations.push(parent);
        }
        locations.reverse();
        locations
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortKey {
    Name,
    Type,
    Size,
    Modified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewPreferences {
    pub show_hidden: bool,
    pub folders_first: bool,
    pub sort_key: SortKey,
    pub sort_direction: SortDirection,
}

impl Default for ViewPreferences {
    fn default() -> Self {
        Self {
            show_hidden: false,
            folders_first: true,
            sort_key: SortKey::Name,
            sort_direction: SortDirection::Ascending,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EntryKind {
    Directory,
    DirectorySymbolicLink,
    File,
    FileSymbolicLink,
    SymbolicLink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataValue<T> {
    Unknown,
    Known(T),
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub location: Location,
    pub native_name: OsString,
    pub display_name: String,
    pub kind: EntryKind,
    pub size: MetadataValue<u64>,
    pub modified_unix_seconds: MetadataValue<i64>,
    pub mode: MetadataValue<u32>,
    pub is_hidden: bool,
}

impl FileEntry {
    pub fn is_directory(&self) -> bool {
        matches!(
            self.kind,
            EntryKind::Directory | EntryKind::DirectorySymbolicLink
        )
    }

    pub fn is_symbolic_link(&self) -> bool {
        matches!(
            self.kind,
            EntryKind::DirectorySymbolicLink
                | EntryKind::FileSymbolicLink
                | EntryKind::SymbolicLink
        )
    }

    pub fn is_broken_symbolic_link(&self) -> bool {
        self.kind == EntryKind::SymbolicLink
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FolderColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl FolderColor {
    pub const ALL: [FolderColor; 7] = [
        FolderColor::Red,
        FolderColor::Orange,
        FolderColor::Yellow,
        FolderColor::Green,
        FolderColor::Blue,
        FolderColor::Purple,
        FolderColor::Gray,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Gray => "Gray",
        }
    }

    pub fn hex(self) -> &'static str {
        match self {
            Self::Red => "#e5484d",
            Self::Orange => "#f76b15",
            Self::Yellow => "#e5a50a",
            Self::Green => "#30a46c",
            Self::Blue => "#0090ff",
            Self::Purple => "#8e4ec6",
            Self::Gray => "#8b8d98",
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Red => "folder-color-red",
            Self::Orange => "folder-color-orange",
            Self::Yellow => "folder-color-yellow",
            Self::Green => "folder-color-green",
            Self::Blue => "folder-color-blue",
            Self::Purple => "folder-color-purple",
            Self::Gray => "folder-color-gray",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "red" => Some(Self::Red),
            "orange" => Some(Self::Orange),
            "yellow" => Some(Self::Yellow),
            "green" => Some(Self::Green),
            "blue" => Some(Self::Blue),
            "purple" => Some(Self::Purple),
            "gray" | "grey" => Some(Self::Gray),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum FolderColorValue {
    Preset(FolderColor),
    Custom(String),
}

impl FolderColorValue {
    pub fn hex(&self) -> &str {
        match self {
            Self::Preset(color) => color.hex(),
            Self::Custom(hex) => hex.as_str(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if let Some(preset) = FolderColor::from_name(trimmed) {
            Some(Self::Preset(preset))
        } else if trimmed.starts_with('#')
            && (trimmed.len() == 7 || trimmed.len() == 4 || trimmed.len() == 9)
            && trimmed[1..].chars().all(|c| c.is_ascii_hexdigit())
        {
            Some(Self::Custom(trimmed.to_ascii_lowercase()))
        } else {
            None
        }
    }

    pub fn to_preference_string(&self) -> String {
        match self {
            Self::Preset(preset) => preset.name().to_ascii_lowercase(),
            Self::Custom(hex) => hex.to_ascii_lowercase(),
        }
    }
}

#[cfg(test)]
mod tests;
