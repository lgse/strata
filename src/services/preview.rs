// SPDX-License-Identifier: MIT

use std::{
    borrow::Cow,
    ffi::OsStr,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::model::FileEntry;

use super::{DocumentLayout, LoadHandle};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewRequestId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaPreviewSize {
    pub width: i32,
    pub height: i32,
}

impl MediaPreviewSize {
    pub const MAX_EDGE: i32 = 1280;

    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width: width.clamp(16, Self::MAX_EDGE),
            height: height.clamp(16, Self::MAX_EDGE),
        }
    }

    pub fn for_viewport(width: i32, height: i32, scale: i32) -> Self {
        Self::new(width.saturating_mul(scale), height.saturating_mul(scale))
    }
}

#[derive(Clone, Debug)]
pub struct PreviewRequest {
    pub id: PreviewRequestId,
    pub entry: FileEntry,
    pub text_byte_limit: usize,
    pub render_document: bool,
    pub pdf_page: i32,
    pub media_size: MediaPreviewSize,
}

/// A decode request, not a playable file. Only the sandbox may open `path`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxedMedia {
    pub(crate) path: PathBuf,
    pub(crate) size: MediaPreviewSize,
    pub(crate) backend: crate::sandbox::MediaPreviewBackend,
    pub(crate) input_owner: Option<PreviewInputLease>,
}

impl SandboxedMedia {
    pub(crate) fn retain_input(mut self, owner: impl Send + Sync + 'static) -> Self {
        self.input_owner = Some(PreviewInputLease(std::sync::Arc::new(owner)));
        self
    }
}

/// Keeps a staged source alive across player clones, seeks, and worker teardown.
#[derive(Clone)]
pub(crate) struct PreviewInputLease(std::sync::Arc<dyn Send + Sync>);

impl std::fmt::Debug for PreviewInputLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PreviewInputLease")
    }
}

impl PartialEq for PreviewInputLease {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for PreviewInputLease {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewContent {
    Text {
        content: String,
        truncated: bool,
    },
    Document {
        source: String,
        document: Option<DocumentLayout>,
        fallback_reason: Option<String>,
        warnings: Vec<String>,
        truncated: bool,
    },
    Workbook {
        document: DocumentLayout,
        warnings: Vec<String>,
    },
    Image,
    Media,
    Rasterized {
        png: Vec<u8>,
    },
    SandboxedMedia {
        media: SandboxedMedia,
    },
    Pdf {
        png: Vec<u8>,
        page: i32,
        pages: i32,
    },
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub request_id: PreviewRequestId,
    pub entry: FileEntry,
    pub content_type: String,
    pub content: PreviewContent,
}

#[derive(Clone, Debug)]
pub enum PreviewEvent {
    Ready(Preview),
    Failed {
        request_id: PreviewRequestId,
        entry: FileEntry,
        message: String,
    },
}

pub trait PreviewProvider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle;
}

fn content_type_for_path(path: &Path) -> glib::GString {
    gio::content_type_guess(Some(path), None::<&[u8]>).0
}

pub(crate) fn is_image_path(path: &Path) -> bool {
    content_type_for_path(path).starts_with("image/")
}

pub(crate) fn is_media_path(path: &Path) -> bool {
    let content_type = content_type_for_path(path);
    content_type.starts_with("audio/") || content_type.starts_with("video/")
}

pub(crate) fn supports_remote_video(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "mov" | "mp4"))
}

pub(crate) fn has_plain_text_extension(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "conf" | "ini"))
}

pub(crate) fn normalize_preview_text(text: &str) -> Cow<'_, str> {
    if text.contains('\0') {
        Cow::Owned(text.replace('\0', "�"))
    } else {
        Cow::Borrowed(text)
    }
}

pub(crate) fn is_extensionless_dotfile(name: &OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    bytes.len() > 1 && bytes.starts_with(b".") && Path::new(name).extension().is_none()
}

pub(crate) fn is_non_executable_extensionless_dotfile(
    name: &OsStr,
    unix_mode: Option<u32>,
) -> bool {
    is_extensionless_dotfile(name) && unix_mode.is_some_and(|mode| mode & 0o111 == 0)
}

pub(crate) fn content_family(content_type: &str) -> PreviewContent {
    if content_type == "application/pdf" {
        PreviewContent::Pdf {
            png: Vec::new(),
            page: 0,
            pages: 0,
        }
    } else if content_type == "image/gif" {
        PreviewContent::Media
    } else if content_type.starts_with("image/") {
        PreviewContent::Image
    } else if content_type.starts_with("audio/") || content_type.starts_with("video/") {
        PreviewContent::Media
    } else if content_type.starts_with("text/")
        || matches!(
            content_type,
            "application/json"
                | "application/ld+json"
                | "application/toml"
                | "application/x-yaml"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/x-shellscript"
        )
        || content_type.ends_with("+json")
        || content_type.ends_with("+xml")
    {
        PreviewContent::Text {
            content: String::new(),
            truncated: false,
        }
    } else {
        PreviewContent::Unsupported
    }
}

#[cfg(test)]
mod tests;
