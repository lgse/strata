// SPDX-License-Identifier: MIT

use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};

use super::DocumentMedia;
use crate::sandbox::{self, Cancellation, MediaPreviewBackend, ParseOperation};

pub(crate) const IMAGE_INPUT_LIMIT: u64 = 8 * 1024 * 1024;
pub(crate) const DIAGRAM_INPUT_LIMIT: usize = 16 * 1024;
pub(crate) const DOCUMENT_MEDIA_LIMIT: usize = 16;

pub(crate) fn render(
    source: &DocumentMedia,
    document_path: Option<&Path>,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    let (bytes, suffix, operation) = match source {
        DocumentMedia::Image(destination) => {
            let parent = document_path
                .and_then(Path::parent)
                .ok_or("Local images are unavailable for remote documents")?;
            let (bytes, suffix) = read_image(parent, destination)?;
            (bytes, suffix, ParseOperation::DocumentImage)
        }
        DocumentMedia::Mermaid(source) => {
            if source.len() > DIAGRAM_INPUT_LIMIT || source.lines().count() > 256 {
                return Err("Mermaid diagram exceeds the 16 KB / 256-line preview limit".into());
            }
            (
                source.as_bytes().to_vec(),
                ".mmd".into(),
                ParseOperation::DocumentMermaid,
            )
        }
    };
    let mut input = tempfile::Builder::new()
        .prefix("strata-document-")
        .suffix(&suffix)
        .tempfile()
        .map_err(|error| error.to_string())?;
    input.write_all(&bytes).map_err(|error| error.to_string())?;
    sandbox::parse(
        input.path(),
        operation,
        0,
        MediaPreviewBackend::Software,
        cancellation,
    )
    .map(|output| output.data)
}

fn read_image(parent: &Path, destination: &str) -> Result<(Vec<u8>, String), String> {
    if destination.contains(':') || destination.starts_with('/') {
        return Err(
            "Only relative local images are loaded; remote images are blocked for privacy".into(),
        );
    }
    let path = destination.split(['?', '#']).next().unwrap_or_default();
    let decoded =
        glib::uri_unescape_string(path, None::<&str>).ok_or("Invalid image URL encoding")?;
    let path = Path::new(decoded.as_str());
    if path.is_absolute() || path.as_os_str().is_empty() {
        return Err("Image must be inside the document's folder".into());
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg"
    ) {
        return Err("Unsupported image format".into());
    }
    let root = File::open(parent).map_err(|_| "Cannot open the document's folder")?;
    // Resolve and open atomically beneath this directory, including symlink targets.
    let fd = openat2(
        &root,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| "Image is missing or outside the document's folder")?;
    let file = File::from(fd);
    let metadata = file.metadata().map_err(|_| "Cannot inspect image")?;
    if !metadata.is_file() || metadata.len() > IMAGE_INPUT_LIMIT {
        return Err("Image must be a regular file no larger than 8 MB".into());
    }
    let mut bytes = Vec::new();
    file.take(IMAGE_INPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read image")?;
    if bytes.len() as u64 > IMAGE_INPUT_LIMIT {
        return Err("Image exceeds the 8 MB preview limit".into());
    }
    Ok((bytes, format!(".{extension}")))
}

#[cfg(test)]
mod tests;
