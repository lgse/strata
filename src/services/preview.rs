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
pub struct DatabaseTableItem {
    pub name: String,
    pub is_view: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseColumn {
    pub name: String,
    pub decl_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseTableData {
    pub name: String,
    pub is_view: bool,
    pub schema: String,
    pub columns: Vec<DatabaseColumn>,
    pub total_rows: Option<usize>,
    pub rows_csv: String,
    pub page: usize,
}

/// Rows per database page. Mirrors `MAX_DATABASE_ROWS` in the sandbox helper.
pub const DATABASE_PAGE_SIZE: usize = 50;

/// Stride separating the table index from the page number when both are packed
/// into the single `pdf_page` request field. Large enough for ~5M rows per table.
pub const DATABASE_PAGE_STRIDE: i32 = 100_000;

pub fn encode_database_page(table: usize, page: usize) -> i32 {
    (table
        .saturating_mul(DATABASE_PAGE_STRIDE as usize)
        .saturating_add(page)) as i32
}

pub fn decode_database_page(value: i32) -> Option<(usize, usize)> {
    if value < 0 {
        return None;
    }
    let stride = DATABASE_PAGE_STRIDE as usize;
    Some(((value as usize) / stride, (value as usize) % stride))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewContent {
    Text {
        content: String,
        truncated: bool,
    },
    Image,
    Media,
    Rasterized {
        png: Vec<u8>,
    },
    SandboxedMedia {
        data: Vec<u8>,
    },
    Pdf {
        png: Vec<u8>,
        page: i32,
        pages: i32,
    },
    Database {
        tables: Vec<DatabaseTableItem>,
        selected: Option<DatabaseTableData>,
    },
    DatabaseTable(DatabaseTableData),
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

pub(crate) fn has_database_extension(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "db" | "sqlite" | "sqlite3" | "db3" | "s3db" | "sl3"
            )
        })
}

fn is_sqlite_content_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "application/vnd.sqlite3"
            | "application/x-sqlite3"
            | "application/sqlite3"
            | "application/x-sqlite"
            | "application/vnd.sqlite"
    )
}

pub(crate) fn content_family(content_type: &str) -> PreviewContent {
    if content_type == "application/pdf" {
        PreviewContent::Pdf {
            png: Vec::new(),
            page: 0,
            pages: 0,
        }
    } else if is_sqlite_content_type(content_type) {
        PreviewContent::Database {
            tables: Vec::new(),
            selected: None,
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
