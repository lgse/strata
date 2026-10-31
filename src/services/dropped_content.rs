// SPDX-License-Identifier: MIT

//! Files saved from content another application drops or pastes, rather than
//! from existing files. Nothing here touches the network: a dropped address
//! becomes a link file, never a download.

use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use glib::{KeyFile, KeyFileFlags, Uri, UriFlags};
use image::ImageFormat;

#[cfg(test)]
mod tests;

/// Bounds what a browser can make Strata hold in memory for one dropped file.
pub(crate) const MAX_DROPPED_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ADDRESS_BYTES: usize = 16 * 1024;
const MAX_STEM_BYTES: usize = 200;
const DESKTOP_GROUP: &str = "Desktop Entry";
const LINK_EXTENSION: &str = "desktop";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WebLink {
    address: String,
    name: String,
}

impl WebLink {
    /// Accepts one credential-free HTTP(S) address; anything else is not a link.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty()
            || text.len() > MAX_ADDRESS_BYTES
            || text.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return None;
        }
        let uri = Uri::parse(text, UriFlags::ENCODED).ok()?;
        if !matches!(uri.scheme().to_ascii_lowercase().as_str(), "http" | "https")
            || uri.userinfo().is_some()
        {
            return None;
        }
        let host = uri.host().filter(|host| !host.is_empty())?;
        let host = host.strip_prefix("www.").unwrap_or(&host);
        Some(Self {
            address: text.to_owned(),
            name: file_stem_from(host)?,
        })
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// A freedesktop link entry has no `Exec` key, so opening it cannot run a program.
    fn desktop_entry(&self) -> Vec<u8> {
        let entry = KeyFile::new();
        entry.set_string(DESKTOP_GROUP, "Type", "Link");
        entry.set_string(DESKTOP_GROUP, "Name", &self.name);
        entry.set_string(DESKTOP_GROUP, "URL", &self.address);
        entry.set_string(DESKTOP_GROUP, "Icon", "text-html");
        entry.to_data().as_bytes().to_vec()
    }

    /// Reads back a link entry; launchers and other addresses are not web links.
    pub(crate) fn from_desktop_entry(data: &str) -> Option<Self> {
        let entry = KeyFile::new();
        entry.load_from_data(data, KeyFileFlags::NONE).ok()?;
        if entry.string(DESKTOP_GROUP, "Type").ok()? != "Link"
            || entry.has_key(DESKTOP_GROUP, "Exec").unwrap_or(true)
        {
            return None;
        }
        Self::parse(&entry.string(DESKTOP_GROUP, "URL").ok()?)
    }
}

/// Whether a name is one this module would give a saved link.
pub(crate) fn has_link_extension(name: &Path) -> bool {
    name.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(LINK_EXTENSION))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DroppedFile {
    stem: String,
    extension: &'static str,
    bytes: Vec<u8>,
}

impl DroppedFile {
    pub(crate) fn text(stem: &str, text: String) -> Self {
        Self {
            stem: stem.to_owned(),
            extension: "txt",
            bytes: text.into_bytes(),
        }
    }

    pub(crate) fn png(stem: &str, bytes: Vec<u8>) -> Self {
        Self {
            stem: stem.to_owned(),
            extension: "png",
            bytes,
        }
    }

    pub(crate) fn link(link: &WebLink) -> Self {
        Self {
            stem: link.name.clone(),
            extension: LINK_EXTENSION,
            bytes: link.desktop_entry(),
        }
    }

    /// Keeps a browser's encoded image as it is. The extension always follows
    /// the bytes, so a supplied name cannot turn an image into another file type.
    pub(crate) fn image(name: Option<&str>, fallback_stem: &str, bytes: Vec<u8>) -> Option<Self> {
        let extension = web_image_extension(&bytes)?;
        let stem = name
            .and_then(|name| Path::new(name).file_stem()?.to_str())
            .and_then(file_stem_from)
            .unwrap_or_else(|| fallback_stem.to_owned());
        Some(Self {
            stem,
            extension,
            bytes,
        })
    }

    #[cfg(test)]
    pub(crate) fn name(&self) -> String {
        format!("{}.{}", self.stem, self.extension)
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn web_image_extension(bytes: &[u8]) -> Option<&'static str> {
    let format = image::guess_format(bytes).ok()?;
    matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Avif
            | ImageFormat::Bmp
            | ImageFormat::Tiff
            | ImageFormat::Ico
    )
    .then(|| format.extensions_str().first().copied())
    .flatten()
}

/// Turns untrusted text into a visible, single-component file stem.
fn file_stem_from(text: &str) -> Option<String> {
    let mut stem = String::new();
    for c in text
        .trim()
        .trim_start_matches('.')
        .chars()
        .map(|c| if c == '/' || c.is_control() { '-' } else { c })
    {
        if stem.len() + c.len_utf8() > MAX_STEM_BYTES {
            break;
        }
        stem.push(c);
    }
    let stem = stem.trim_end().to_owned();
    (!stem.is_empty()).then_some(stem)
}

/// Saves every file under an unused name, in order, stopping at the first failure.
pub(crate) fn save_dropped_files(
    directory: &Path,
    files: &[DroppedFile],
) -> io::Result<Vec<PathBuf>> {
    files
        .iter()
        .map(|file| save_new_file(directory, &file.stem, file.extension, &file.bytes))
        .collect()
}

/// Publishes the complete contents under the first unused numbered name. Existing
/// entries, including dangling symbolic links, are never replaced or followed.
pub(crate) fn save_new_file(
    directory: &Path,
    stem: &str,
    extension: &str,
    bytes: &[u8],
) -> io::Result<PathBuf> {
    let mut temporary = tempfile::Builder::new()
        .prefix(".strata-drop-")
        .permissions(std::fs::Permissions::from_mode(0o666))
        .tempfile_in(directory)?;
    temporary.write_all(bytes)?;
    for path in numbered_paths(directory, stem, extension) {
        match temporary.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                temporary = error.file;
            }
            Err(error) if cannot_rename_without_replacing(&error.error) => {
                drop(error.file);
                return write_new_file(directory, stem, extension, bytes);
            }
            Err(error) => return Err(error.error),
        }
    }
    Err(io::Error::other("file name suffixes exhausted"))
}

/// Publishing without replacing needs `RENAME_NOREPLACE` or hard links, which
/// some FUSE mounts, such as rclone and MTP drivers, support neither of.
fn cannot_rename_without_replacing(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied | io::ErrorKind::InvalidInput
    )
}

/// Writes in place under an exclusively created name, so the file is visible
/// while it is written; a failed write removes it again.
pub(super) fn write_new_file(
    directory: &Path,
    stem: &str,
    extension: &str,
    bytes: &[u8],
) -> io::Result<PathBuf> {
    for path in numbered_paths(directory, stem, extension) {
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = file.write_all(bytes) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        return Ok(path);
    }
    Err(io::Error::other("file name suffixes exhausted"))
}

fn numbered_paths<'a>(
    directory: &'a Path,
    stem: &'a str,
    extension: &'a str,
) -> impl Iterator<Item = PathBuf> + 'a {
    (0u64..).map(move |suffix| {
        directory.join(if suffix == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem} ({suffix}).{extension}")
        })
    })
}
