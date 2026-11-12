// SPDX-License-Identifier: MIT
//! The sandboxed operations and the formats and sizes they carry.

use std::path::Path;

use super::{MAX_ARCHIVE_COVER_INPUT_BYTES, MAX_RASTER_INPUT_BYTES};
use crate::{
    model::ArchiveFormat,
    services::{
        MediaPreviewSize, ModelFormat, ModelRender, SecretString,
        model_preview::MAX_MODEL_INPUT_BYTES,
    },
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MediaPreviewBackend {
    Automatic,
    VaApi,
    Vulkan,
    Software,
}

impl MediaPreviewBackend {
    pub fn argument(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::VaApi => "vaapi",
            Self::Vulkan => "vulkan",
            Self::Software => "software",
        }
    }

    pub fn from_argument(value: &str) -> Option<Self> {
        match value {
            "automatic" => Some(Self::Automatic),
            "vaapi" => Some(Self::VaApi),
            "vulkan" => Some(Self::Vulkan),
            "software" => Some(Self::Software),
            _ => None,
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfPreviewError {
    PasswordProtected,
}

pub const PDF_PASSWORD_PROTECTED: &str = "This PDF is password protected.";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfRenderSize {
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl PdfRenderSize {
    const MAX_WIDTH: i32 = 1_400;
    const MAX_HEIGHT: i32 = 1_800;
    const MAX_PIXELS: u64 = 2_500_000;

    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width: width.clamp(16, Self::MAX_WIDTH),
            height: height.clamp(16, Self::MAX_HEIGHT),
        }
    }

    pub fn for_viewport_width(width: i32) -> Self {
        Self::new(width, Self::MAX_HEIGHT)
    }

    pub fn image_limits(self) -> (u32, u32, u64) {
        let size = Self::new(self.width, self.height);
        let width = size.width as u32;
        let height = size.height as u32;
        (
            width,
            height,
            (u64::from(width) * u64::from(height)).min(Self::MAX_PIXELS),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverFormat {
    Cbz,
    Cbr,
    Epub,
}

impl CoverFormat {
    pub fn for_name(name: &std::ffi::OsStr) -> Option<Self> {
        let extension = Path::new(name).extension()?.to_str()?;
        Self::from_argument(&extension.to_ascii_lowercase())
    }

    pub fn argument(self) -> &'static str {
        match self {
            Self::Cbz => "cbz",
            Self::Cbr => "cbr",
            Self::Epub => "epub",
        }
    }

    pub fn from_argument(value: &str) -> Option<Self> {
        match value {
            "cbz" => Some(Self::Cbz),
            "cbr" => Some(Self::Cbr),
            "epub" => Some(Self::Epub),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentFormat {
    Docx,
    Odt,
}

impl DocumentFormat {
    pub fn for_file(content_type: &str, name: &std::ffi::OsStr) -> Option<Self> {
        if crate::services::docx::is_document(content_type, name) {
            Some(Self::Docx)
        } else if crate::services::odt::is_document(content_type, name) {
            Some(Self::Odt)
        } else {
            None
        }
    }

    pub fn argument(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Odt => "odt",
        }
    }

    pub fn from_argument(value: &str) -> Option<Self> {
        match value {
            "docx" => Some(Self::Docx),
            "odt" => Some(Self::Odt),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseOperation {
    ThumbnailImage,
    ThumbnailRaw,
    ThumbnailPdf,
    ThumbnailVideo,
    ThumbnailAppImage,
    ThumbnailModel(ModelFormat),
    ThumbnailCover(CoverFormat),
    PreviewCover(CoverFormat),
    PreviewImage,
    PreviewRaw,
    InspectImage,
    ConvertImage,
    DocumentImage,
    DocumentMermaid,
    DocumentMath {
        display: bool,
    },
    MediaMetadata,
    RawMetadata,
    PreviewWorkbook,
    PreviewDocument(DocumentFormat),
    PreviewPdf(PdfRenderSize),
    PreviewModel(ModelRender),
    PreviewRoute(crate::services::route_preview::RouteRender),
    PreviewMedia(MediaPreviewSize),
    PreviewAudio(MediaPreviewSize),
    AudioPeaks,
    VideoStoryboard {
        cell_edge: u32,
    },
    AudioTags,
    AudioCover,
    ArchiveList {
        format: ArchiveFormat,
        password: Option<SecretString>,
    },
}

impl ParseOperation {
    pub(super) fn argument(&self) -> &'static str {
        match self {
            Self::ThumbnailImage => "thumbnail-image",
            Self::ThumbnailRaw => "thumbnail-raw",
            Self::ThumbnailPdf => "thumbnail-pdf",
            Self::ThumbnailVideo => "thumbnail-video",
            Self::ThumbnailAppImage => "thumbnail-appimage",
            Self::ThumbnailModel(_) => "thumbnail-model",
            Self::ThumbnailCover(_) => "thumbnail-cover",
            Self::PreviewCover(_) => "preview-cover",
            Self::PreviewImage => "preview-image",
            Self::PreviewRaw => "preview-raw",
            Self::InspectImage => "inspect-image",
            Self::ConvertImage => "convert-image",
            Self::DocumentImage => "document-image",
            Self::DocumentMermaid => "document-mermaid",
            Self::DocumentMath { display: true } => "document-math",
            Self::DocumentMath { display: false } => "document-inline-math",
            Self::MediaMetadata => "media-metadata",
            Self::RawMetadata => "raw-metadata",
            Self::PreviewWorkbook => "preview-workbook",
            Self::PreviewDocument(_) => "preview-document",
            Self::PreviewPdf(_) => "preview-pdf",
            Self::PreviewModel(_) => "preview-model",
            Self::PreviewRoute(_) => "preview-route",
            Self::PreviewMedia(_) => "preview-media",
            Self::PreviewAudio(_) => "preview-audio",
            Self::AudioPeaks => "audio-peaks",
            Self::VideoStoryboard { .. } => "video-storyboard",
            Self::AudioTags => "audio-tags",
            Self::AudioCover => "audio-cover",
            Self::ArchiveList { .. } => "archive-list",
        }
    }

    pub(super) fn is_media(&self) -> bool {
        matches!(
            self,
            Self::PreviewMedia(_)
                | Self::PreviewAudio(_)
                | Self::AudioPeaks
                | Self::VideoStoryboard { .. }
        )
    }

    pub(super) fn needs_media_libraries(&self) -> bool {
        matches!(
            self,
            Self::ThumbnailVideo
                | Self::PreviewMedia(_)
                | Self::PreviewAudio(_)
                | Self::AudioPeaks
                | Self::VideoStoryboard { .. }
                | Self::AudioTags
                | Self::AudioCover
                | Self::MediaMetadata
        )
    }

    pub(super) fn output_name(&self) -> &'static str {
        if matches!(
            self,
            Self::MediaMetadata
                | Self::AudioTags
                | Self::RawMetadata
                | Self::PreviewWorkbook
                | Self::PreviewDocument(_)
                | Self::PreviewRoute(_)
                | Self::InspectImage
        ) {
            "result.json"
        } else if self.is_media() {
            "result.media"
        } else if matches!(self, Self::ArchiveList { .. }) {
            "result.archive.json"
        } else {
            "result.png"
        }
    }

    pub(super) fn image_limits(&self) -> Option<(u32, u32, u64)> {
        match self {
            Self::ThumbnailImage
            | Self::ThumbnailRaw
            | Self::ThumbnailPdf
            | Self::ThumbnailVideo
            | Self::ThumbnailAppImage
            | Self::ThumbnailModel(_)
            | Self::ThumbnailCover(_) => Some((256, 256, 256 * 256)),
            Self::PreviewCover(_) | Self::AudioCover => Some((800, 800, 800 * 800)),
            Self::ConvertImage => Some((
                crate::services::image_conversion::MAX_EDGE,
                crate::services::image_conversion::MAX_EDGE,
                crate::services::image_conversion::MAX_PIXELS,
            )),
            Self::PreviewImage
            | Self::PreviewRaw
            | Self::DocumentImage
            | Self::DocumentMermaid
            | Self::DocumentMath { .. } => Some((800, 800, 800 * 800)),
            Self::PreviewPdf(size) => Some(size.image_limits()),
            Self::PreviewModel(render) => {
                let size = MediaPreviewSize::new(render.size.width, render.size.height);
                Some((size.width as u32, size.height as u32, 1280 * 1280))
            }
            Self::PreviewMedia(_)
            | Self::PreviewRoute(_)
            | Self::InspectImage
            | Self::PreviewAudio(_)
            | Self::AudioPeaks
            | Self::VideoStoryboard { .. }
            | Self::AudioTags
            | Self::MediaMetadata
            | Self::RawMetadata
            | Self::PreviewWorkbook
            | Self::PreviewDocument(_)
            | Self::ArchiveList { .. } => None,
        }
    }

    pub(super) fn input_size_limit(&self) -> Option<u64> {
        match self {
            Self::ThumbnailImage
            | Self::ThumbnailRaw
            | Self::ThumbnailPdf
            | Self::PreviewImage
            | Self::PreviewRaw
            | Self::RawMetadata
            | Self::PreviewPdf(_) => Some(MAX_RASTER_INPUT_BYTES),
            Self::PreviewModel(render) if render.format != ModelFormat::Gcode => {
                Some(MAX_MODEL_INPUT_BYTES)
            }
            Self::ThumbnailModel(format) if *format != ModelFormat::Gcode => {
                Some(MAX_MODEL_INPUT_BYTES)
            }
            Self::PreviewModel(_) | Self::ThumbnailModel(_) => None,
            Self::ThumbnailCover(_) | Self::PreviewCover(_) => Some(MAX_ARCHIVE_COVER_INPUT_BYTES),
            Self::InspectImage | Self::ConvertImage => {
                Some(crate::services::image_conversion::MAX_INPUT_BYTES)
            }
            Self::PreviewRoute(_) => Some(crate::services::route_preview::MAX_ROUTE_INPUT_BYTES),
            Self::PreviewWorkbook => Some(crate::services::table::WORKBOOK_BYTE_LIMIT),
            Self::PreviewDocument(DocumentFormat::Docx) => {
                Some(crate::services::docx::DOCX_BYTE_LIMIT)
            }
            Self::PreviewDocument(DocumentFormat::Odt) => {
                Some(crate::services::odt::ODT_BYTE_LIMIT)
            }
            Self::DocumentImage => Some(crate::services::document_media::IMAGE_INPUT_LIMIT),
            Self::DocumentMermaid => {
                Some(crate::services::document_media::DIAGRAM_INPUT_LIMIT as u64)
            }
            Self::DocumentMath { .. } => {
                Some(crate::services::document_media::MATH_INPUT_LIMIT as u64)
            }
            Self::ThumbnailVideo
            | Self::ThumbnailAppImage
            | Self::PreviewMedia(_)
            | Self::PreviewAudio(_)
            | Self::AudioPeaks
            | Self::VideoStoryboard { .. }
            | Self::AudioTags
            | Self::AudioCover
            | Self::MediaMetadata
            | Self::ArchiveList { .. } => None,
        }
    }
}
