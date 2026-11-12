// SPDX-License-Identifier: MIT

//! Which files get thumbnails, and the sandboxed renderer each kind uses.
//!
//! Ported from GTK's `src/ui/thumbnail/kind.rs` at `b4b459dcb`, as new code: nothing under
//! `src/ui` moves. A change to that file on a merge from `main` is a change here too
//! (`docs/rune/parity/packages/A1.md`, "Ported from GTK"). The camera kind is left out: it
//! takes its picture from the camera's preview service, which the Rune side has no part for.

use std::path::Path;

use strata_preview::{
    sandbox::{
        self, Cancellation, CoverFormat, MediaPreviewBackend, ParseOperation, browser::Thumbnail,
        raw_metadata,
    },
    services::{
        MediaPreviewSize, ModelFormat, ModelPalette, ModelRender, ModelRenderPurpose, ModelView,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Image,
    RawImage,
    Pdf,
    Video,
    AppImage,
    EmbeddedModel(ModelFormat),
    RenderedModel,
    Cover(CoverFormat),
}

impl Kind {
    /// A kind whose render is slow: one worker stays free of them, as GTK keeps one.
    pub(crate) fn heavy(self) -> bool {
        !matches!(self, Self::Image | Self::AppImage)
    }
}

pub(crate) fn of(path: &Path) -> Option<Kind> {
    if raw_metadata::is_raw(path) {
        return Some(Kind::RawImage);
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" | "heic"
        | "heif" | "avif" | "jxl" => Some(Kind::Image),
        "pdf" => Some(Kind::Pdf),
        "stl" => Some(Kind::RenderedModel),
        "3mf" => Some(Kind::EmbeddedModel(ModelFormat::ThreeMf)),
        "fcstd" => Some(Kind::EmbeddedModel(ModelFormat::FreeCad)),
        "gcode" | "gco" | "bgcode" => Some(Kind::EmbeddedModel(ModelFormat::Gcode)),
        "appimage" => Some(Kind::AppImage),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ogv" => {
            Some(Kind::Video)
        }
        // FFmpeg exposes no art stream for tag-only Ogg, Opus or WAV covers.
        "mp3" | "flac" | "m4a" | "m4b" | "mka" | "aiff" | "aif" | "wma" => Some(Kind::Video),
        _ => CoverFormat::from_argument(&extension).map(Kind::Cover),
    }
}

/// Renders `path`'s thumbnail in the sandbox, as a PNG of at most 256 px.
pub(crate) fn render(
    path: &Path,
    kind: Kind,
    cancellation: &Cancellation,
) -> Result<Thumbnail, String> {
    let operation = match kind {
        Kind::RenderedModel => {
            let rendered = sandbox::parse(
                path,
                ParseOperation::PreviewModel(ModelRender {
                    purpose: ModelRenderPurpose::Thumbnail,
                    format: ModelFormat::Stl,
                    size: MediaPreviewSize::new(256, 256),
                    // Shared thumbnail-cache artwork must not depend on the active theme.
                    palette: ModelPalette {
                        accent: 0xcccccc,
                        surface: 0,
                    },
                    view: ModelView::default(),
                }),
                256,
                MediaPreviewBackend::Software,
                cancellation,
            )?;
            return Ok(Thumbnail {
                png: rendered.data,
                metadata: None,
            });
        }
        Kind::Image => ParseOperation::ThumbnailImage,
        Kind::RawImage => ParseOperation::ThumbnailRaw,
        Kind::Pdf => ParseOperation::ThumbnailPdf,
        Kind::Video => ParseOperation::ThumbnailVideo,
        Kind::AppImage => ParseOperation::ThumbnailAppImage,
        Kind::EmbeddedModel(format) => ParseOperation::ThumbnailModel(format),
        Kind::Cover(format) => ParseOperation::ThumbnailCover(format),
    };
    sandbox::browser::thumbnail(path, operation, cancellation)
}

#[cfg(test)]
mod tests;
