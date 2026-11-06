// SPDX-License-Identifier: MIT

//! Which files get thumbnails, and the sandboxed renderer each kind uses.

use std::path::Path;

use crate::sandbox::{Cancellation, ParseOperation};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ThumbnailKind {
    Camera,
    Image,
    RawImage,
    Pdf,
    Video,
    AppImage,
    EmbeddedModel(crate::services::ModelFormat),
    RenderedModel,
    Cover(crate::sandbox::CoverFormat),
}

pub(super) fn heavy(kind: ThumbnailKind) -> bool {
    matches!(
        kind,
        ThumbnailKind::RawImage
            | ThumbnailKind::Pdf
            | ThumbnailKind::Video
            | ThumbnailKind::EmbeddedModel(_)
            | ThumbnailKind::RenderedModel
            | ThumbnailKind::Cover(_)
    )
}

pub(super) fn thumbnail_kind(path: &Path) -> Option<ThumbnailKind> {
    if crate::sandbox::raw_metadata::is_raw(path) {
        return Some(ThumbnailKind::RawImage);
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" | "heic"
        | "heif" | "avif" | "jxl" => Some(ThumbnailKind::Image),
        "pdf" => Some(ThumbnailKind::Pdf),
        "stl" => Some(ThumbnailKind::RenderedModel),
        "3mf" => Some(ThumbnailKind::EmbeddedModel(
            crate::services::ModelFormat::ThreeMf,
        )),
        "fcstd" => Some(ThumbnailKind::EmbeddedModel(
            crate::services::ModelFormat::FreeCad,
        )),
        "gcode" | "gco" | "bgcode" => Some(ThumbnailKind::EmbeddedModel(
            crate::services::ModelFormat::Gcode,
        )),
        "appimage" => Some(ThumbnailKind::AppImage),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ogv" => {
            Some(ThumbnailKind::Video)
        }
        // FFmpeg exposes no art stream for tag-only Ogg, Opus or WAV covers.
        "mp3" | "flac" | "m4a" | "m4b" | "mka" | "aiff" | "aif" | "wma" => {
            Some(ThumbnailKind::Video)
        }
        _ => crate::sandbox::CoverFormat::from_argument(&extension).map(ThumbnailKind::Cover),
    }
}

pub(super) fn render_thumbnail(
    path: &Path,
    kind: ThumbnailKind,
    cancellation: &Cancellation,
) -> Result<crate::sandbox::browser::Thumbnail, String> {
    let operation = match kind {
        ThumbnailKind::RenderedModel => {
            let rendered = crate::sandbox::parse(
                path,
                ParseOperation::PreviewModel(crate::services::ModelRender {
                    purpose: crate::services::ModelRenderPurpose::Thumbnail,
                    format: crate::services::ModelFormat::Stl,
                    size: crate::services::MediaPreviewSize::new(256, 256),
                    // Shared thumbnail-cache artwork must not depend on the active theme.
                    palette: crate::services::ModelPalette {
                        accent: 0xcccccc,
                        surface: 0,
                    },
                    view: crate::services::ModelView::default(),
                }),
                256,
                crate::sandbox::MediaPreviewBackend::Software,
                cancellation,
            )?;
            return Ok(crate::sandbox::browser::Thumbnail {
                png: rendered.data,
                metadata: None,
            });
        }
        ThumbnailKind::Camera => return Err("Camera thumbnails require their preview icon".into()),
        ThumbnailKind::Image => ParseOperation::ThumbnailImage,
        ThumbnailKind::RawImage => ParseOperation::ThumbnailRaw,
        ThumbnailKind::Pdf => ParseOperation::ThumbnailPdf,
        ThumbnailKind::Video => ParseOperation::ThumbnailVideo,
        ThumbnailKind::AppImage => ParseOperation::ThumbnailAppImage,
        ThumbnailKind::EmbeddedModel(format) => ParseOperation::ThumbnailModel(format),
        ThumbnailKind::Cover(format) => ParseOperation::ThumbnailCover(format),
    };
    crate::sandbox::browser::thumbnail(path, operation, cancellation)
}
