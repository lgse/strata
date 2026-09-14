// SPDX-License-Identifier: MIT

use std::{
    cmp::Ordering,
    ffi::OsString,
    path::{Path, PathBuf},
};

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
            LocationKind::Uri(uri) if uri.starts_with("strata-search://") => None,
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
        match (&self.kind, &from.kind, &to.kind) {
            (LocationKind::Native(path), LocationKind::Native(from), LocationKind::Native(to)) => {
                let suffix = path.strip_prefix(from).ok()?;
                Some(Self::local(if suffix.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(suffix)
                }))
            }
            (LocationKind::Uri(uri), LocationKind::Uri(from), LocationKind::Uri(to)) => {
                let file = gio::File::for_uri(uri);
                let from = gio::File::for_uri(from);
                let to = gio::File::for_uri(to);
                let relocated = if file.equal(&from) {
                    to
                } else {
                    to.resolve_relative_path(from.relative_path(&file)?)
                };
                Some(Self::uri(relocated.uri()))
            }
            _ => None,
        }
    }

    pub fn is_within(&self, other: &Self) -> bool {
        if let Some((path, parent)) = self.native_path().zip(other.native_path()) {
            return path.starts_with(parent);
        }
        let (Some(uri), Some(parent_uri)) = (self.uri_value(), other.uri_value()) else {
            return false;
        };
        let file = gio::File::for_uri(uri);
        let parent = gio::File::for_uri(parent_uri);
        file.equal(&parent) || file.has_prefix(&parent)
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

    pub fn is_camera_photo_root(&self) -> bool {
        self.uri_value().is_some_and(|uri| {
            let file = gio::File::for_uri(uri);
            file.has_uri_scheme("gphoto2") && file.parent().is_none()
        })
    }

    pub fn contains_camera_photo_location(&self, location: &Self) -> bool {
        self.is_camera_photo_root()
            && self.uri_value().is_some_and(|uri| {
                // GIO's fallback URI implementation distinguishes a trailing root slash.
                location.is_within(self)
                    || location.is_within(&Self::uri(uri.trim_end_matches('/')))
            })
    }

    pub fn display_name(&self) -> String {
        if self.is_camera_photo_root() {
            return "Photos".into();
        }
        match &self.kind {
            LocationKind::Native(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            LocationKind::Uri(uri) if uri == "trash:///" => "Trash".into(),
            LocationKind::Uri(_) if self.is_smart_folder() => {
                let id = self.smart_folder_id().unwrap_or("Smart Folder");
                match id {
                    "recent-documents" => "Recent Documents".to_string(),
                    "recent-images" => "Recent Images".to_string(),
                    "large-files" => "Large Files (>100MB)".to_string(),
                    _ => id.to_string(),
                }
            }
            LocationKind::Uri(uri) => self
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| {
                    uri.trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or(uri)
                        .into()
                }),
        }
    }

    pub fn breadcrumbs(&self) -> Vec<Location> {
        let mut locations = Vec::new();
        let mut current = self.clone();
        locations.push(current.clone());
        while let Some(parent) = current.parent() {
            current = parent.clone();
            if locations.contains(&parent) {
                break;
            }
            locations.push(parent);
        }
        locations.reverse();
        locations
    }

    pub fn smart_folder(id: &str) -> Self {
        Self::uri(format!("strata-search://{id}"))
    }

    pub fn smart_folder_id(&self) -> Option<&str> {
        self.uri_value()
            .filter(|uri| uri.starts_with("strata-search://"))
            .map(|uri| &uri["strata-search://".len()..])
            .filter(|id| !id.is_empty())
    }

    pub fn is_smart_folder(&self) -> bool {
        self.smart_folder_id().is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FileCategory {
    Any,
    Document,
    Image,
    Audio,
    Video,
    Archive,
    Code,
    Folder,
}

impl FileCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Any => "Any Kind",
            Self::Document => "Documents",
            Self::Image => "Images",
            Self::Audio => "Audio",
            Self::Video => "Videos",
            Self::Archive => "Archives",
            Self::Code => "Code",
            Self::Folder => "Folders",
        }
    }

    pub fn matches(&self, path: &Path, is_directory: bool) -> bool {
        match self {
            Self::Any => true,
            Self::Folder => is_directory,
            Self::Document => {
                !is_directory
                    && Self::has_extension(
                        path,
                        &[
                            "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "markdown", "epub",
                            "pages", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "csv", "tsv",
                        ],
                    )
            }
            Self::Image => {
                !is_directory
                    && Self::has_extension(
                        path,
                        &[
                            "jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico", "tiff",
                            "tif", "avif", "heic", "heif", "raw", "exr", "psd", "xcf",
                        ],
                    )
            }
            Self::Audio => {
                !is_directory
                    && Self::has_extension(
                        path,
                        &[
                            "mp3", "flac", "wav", "ogg", "m4a", "aac", "opus", "wma", "aiff",
                            "alac",
                        ],
                    )
            }
            Self::Video => {
                !is_directory
                    && Self::has_extension(
                        path,
                        &[
                            "mp4", "mkv", "webm", "avi", "mov", "flv", "wmv", "m4v", "ts", "3gp",
                        ],
                    )
            }
            Self::Archive => {
                !is_directory
                    && Self::has_extension(
                        path,
                        &[
                            "zip", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "7z", "rar",
                            "zst", "iso",
                        ],
                    )
            }
            Self::Code => {
                !is_directory
                    && Self::has_extension(
                        path,
                        &[
                            "rs", "c", "cpp", "cc", "cxx", "h", "hpp", "py", "pyw", "js", "mjs",
                            "cjs", "ts", "tsx", "jsx", "html", "htm", "css", "scss", "sass",
                            "less", "json", "toml", "yaml", "yml", "xml", "sh", "bash", "zsh",
                            "fish", "go", "java", "kt", "kts", "swift", "rb", "php", "sql", "lua",
                            "zig",
                        ],
                    )
            }
        }
    }

    fn has_extension(path: &Path, extensions: &[&str]) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                let ext_lower = ext.to_ascii_lowercase();
                extensions.iter().any(|&candidate| candidate == ext_lower)
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DateConstraint {
    WithinPastDays(u32),
    OlderThanDays(u32),
}

impl DateConstraint {
    pub fn matches(&self, modified_secs: u64, now_secs: u64) -> bool {
        if modified_secs == 0 {
            return false;
        }
        let age_secs = now_secs.saturating_sub(modified_secs);
        match self {
            Self::WithinPastDays(days) => age_secs <= u64::from(*days).saturating_mul(86400),
            Self::OlderThanDays(days) => age_secs > u64::from(*days).saturating_mul(86400),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::WithinPastDays(1) => "Past 24 hours".to_string(),
            Self::WithinPastDays(7) => "Past 7 days".to_string(),
            Self::WithinPastDays(30) => "Past 30 days".to_string(),
            Self::WithinPastDays(90) => "Past 90 days".to_string(),
            Self::WithinPastDays(365) => "Past year".to_string(),
            Self::WithinPastDays(days) => format!("Past {days} days"),
            Self::OlderThanDays(days) => format!("Older than {days} days"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SizeConstraint {
    GreaterThan(u64),
    LessThan(u64),
}

impl SizeConstraint {
    pub fn matches(&self, size_bytes: u64, is_directory: bool) -> bool {
        if is_directory {
            return false;
        }
        match self {
            Self::GreaterThan(bytes) => size_bytes > *bytes,
            Self::LessThan(bytes) => size_bytes < *bytes,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::GreaterThan(bytes) => format!("> {}", format_size_human(*bytes)),
            Self::LessThan(bytes) => format!("< {}", format_size_human(*bytes)),
        }
    }
}

fn format_size_human(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{} GB", bytes / 1_000_000_000)
    } else if bytes >= 1_000_000 {
        format!("{} MB", bytes / 1_000_000)
    } else if bytes >= 1_000 {
        format!("{} KB", bytes / 1_000)
    } else {
        format!("{bytes} B")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum SmartQueryRule {
    Kind(FileCategory),
    NameContains(String),
    DateModified(DateConstraint),
    FileSize(SizeConstraint),
}

impl SmartQueryRule {
    pub fn matches(
        &self,
        path: &Path,
        name: &str,
        is_directory: bool,
        size_bytes: u64,
        modified_secs: u64,
        now_secs: u64,
    ) -> bool {
        match self {
            Self::Kind(category) => category.matches(path, is_directory),
            Self::NameContains(sub) => {
                if sub.is_empty() {
                    true
                } else {
                    name.to_ascii_lowercase()
                        .contains(&sub.to_ascii_lowercase())
                }
            }
            Self::DateModified(date) => date.matches(modified_secs, now_secs),
            Self::FileSize(size) => size.matches(size_bytes, is_directory),
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Kind(category) => format!("Kind: {}", category.label()),
            Self::NameContains(sub) => format!("Name: \"{sub}\""),
            Self::DateModified(date) => format!("Modified: {}", date.label()),
            Self::FileSize(size) => format!("Size: {}", size.label()),
        }
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
    /// Physical source for virtual entries; `location` remains their operational identity.
    pub thumbnail_path: Option<PathBuf>,
    pub native_name: OsString,
    pub display_name: String,
    pub kind: EntryKind,
    pub size: MetadataValue<u64>,
    pub modified_unix_seconds: MetadataValue<i64>,
    pub mode: MetadataValue<u32>,
    pub is_hidden: bool,
}

impl FileEntry {
    pub fn local_thumbnail_path(&self) -> Option<&Path> {
        self.location
            .native_path()
            .or(self.thumbnail_path.as_deref())
    }

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
