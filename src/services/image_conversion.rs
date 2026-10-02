// SPDX-License-Identifier: MIT

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::sandbox::{self, Cancellation, MediaPreviewBackend, ParseOperation};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

pub(crate) const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_PIXELS: u64 = 16_000_000;
pub(crate) const MAX_EDGE: u32 = 16_384;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ImageKind {
    Jpeg,
    Bmp,
    Gif,
    WebP,
    Png,
}

impl ImageKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Bmp => "BMP",
            Self::Gif => "GIF",
            Self::WebP => "WebP",
            Self::Png => "PNG",
        }
    }

    pub(crate) fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Jpeg => &["jpg", "jpeg", "jpe"],
            Self::Bmp => &["bmp"],
            Self::Gif => &["gif"],
            Self::WebP => &["webp"],
            Self::Png => &["png"],
        }
    }

    pub(crate) fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Bmp => "image/bmp",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
            Self::Png => "image/png",
        }
    }

    pub(crate) fn format(self) -> image::ImageFormat {
        match self {
            Self::Jpeg => image::ImageFormat::Jpeg,
            Self::Bmp => image::ImageFormat::Bmp,
            Self::Gif => image::ImageFormat::Gif,
            Self::WebP => image::ImageFormat::WebP,
            Self::Png => image::ImageFormat::Png,
        }
    }
}

// Only a bounded signature read happens outside the decoder sandbox.
pub(crate) fn detect(path: &Path) -> Option<ImageKind> {
    let mut header = [0; 32];
    let count = fs::File::open(path).ok()?.read(&mut header).ok()?;
    match image::guess_format(&header[..count]).ok()? {
        image::ImageFormat::Jpeg => Some(ImageKind::Jpeg),
        image::ImageFormat::Bmp => Some(ImageKind::Bmp),
        image::ImageFormat::Gif => Some(ImageKind::Gif),
        image::ImageFormat::WebP => Some(ImageKind::WebP),
        image::ImageFormat::Png => Some(ImageKind::Png),
        _ => None,
    }
}

pub(crate) fn inspect(path: &Path, cancelled: &Cancellation) -> Result<ImageKind, String> {
    let output = sandbox::parse(
        path,
        ParseOperation::InspectImage,
        0,
        MediaPreviewBackend::Software,
        cancelled,
    )?;
    serde_json::from_slice(&output.data).map_err(|_| "Could not identify the image".to_owned())
}

pub(crate) fn convert(
    path: &Path,
    name: &Path,
    cancelled: &Cancellation,
) -> Result<PathBuf, String> {
    let output = sandbox::parse(
        path,
        ParseOperation::ConvertImage,
        0,
        MediaPreviewBackend::Software,
        cancelled,
    )
    .map_err(|error| format!("Could not convert to PNG: {error}. Try another image."))?;
    if cancelled.is_cancelled() {
        return Err("Conversion cancelled".into());
    }
    let directory = tempfile::Builder::new()
        .prefix(super::remote_download::DOWNLOAD_PREFIX)
        .tempdir()
        .map_err(|error| format!("Could not create a temporary folder: {error}"))?;
    let destination = directory
        .path()
        .join(name.file_name().ok_or("Invalid PNG filename")?);
    let mut file = fs::File::create(&destination).map_err(|error| error.to_string())?;
    file.write_all(&output.data)
        .map_err(|error| format!("Could not write the PNG: {error}"))?;
    if cancelled.is_cancelled() {
        return Err("Conversion cancelled".into());
    }
    let _persisted = directory.keep();
    Ok(destination)
}

pub(crate) fn with_extension(path: &Path, extension: &str) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let mut end = stem.len().min(255 - extension.len() - 1);
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    path.with_file_name(format!("{}.{}", &stem[..end], extension))
}
