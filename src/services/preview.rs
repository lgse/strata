// SPDX-License-Identifier: GPL-3.0-or-later

use std::{ffi::OsStr, path::Path, rc::Rc};

use crate::model::FileEntry;

use super::LoadHandle;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewRequestId(pub u64);

#[derive(Clone, Debug)]
pub struct PreviewRequest {
    pub id: PreviewRequestId,
    pub entry: FileEntry,
    pub text_byte_limit: usize,
    pub pdf_page: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewContent {
    Text { content: String, truncated: bool },
    Image,
    Media,
    Rasterized { png: Vec<u8> },
    SandboxedMedia { data: Vec<u8> },
    Pdf { png: Vec<u8>, page: i32, pages: i32 },
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

pub(crate) fn has_plain_text_extension(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "conf" | "ini"))
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
