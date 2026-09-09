// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, path::Path, rc::Rc};

use calamine::Reader;

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
    Text {
        content: String,
        truncated: bool,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
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

pub(crate) fn has_csv_extension(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
}

pub(crate) fn has_tsv_extension(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tsv"))
}

pub(crate) fn has_excel_extension(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "xlsx" | "xls" | "ods"
            )
        })
}

/// Cap on rows parsed for a table preview, matching pandas' truncated-display feel.
pub(crate) const TABLE_ROW_LIMIT: usize = 200;

/// Parses delimited text (CSV comma, TSV tab) into a header row plus up to
/// `TABLE_ROW_LIMIT` data rows. Ragged rows are tolerated (`flexible`);
/// unparsable rows are skipped.
pub(crate) fn parse_delimited_table(
    content: &str,
    delimiter: u8,
) -> (Vec<String>, Vec<Vec<String>>, bool) {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(content.as_bytes());
    let headers = reader
        .headers()
        .map(|record| record.iter().map(str::to_owned).collect())
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut truncated = false;
    for record in reader.records().flatten() {
        if rows.len() >= TABLE_ROW_LIMIT {
            truncated = true;
            break;
        }
        rows.push(record.iter().map(str::to_owned).collect());
    }
    (headers, rows, truncated)
}

/// Bound on workbook file size before attempting to parse it, so a
/// pathological or oversized file can't spend unbounded time/memory unzipping.
pub(crate) const EXCEL_BYTE_LIMIT: u64 = 20 * 1024 * 1024;

type TableParseResult = Result<(Vec<String>, Vec<Vec<String>>, bool), String>;

/// Parses the first worksheet of a local Excel/ODS workbook into a header row
/// plus up to `TABLE_ROW_LIMIT` data rows.
pub(crate) fn parse_excel_table(path: &Path) -> TableParseResult {
    let size = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len();
    if size > EXCEL_BYTE_LIMIT {
        return Err("Workbook is too large to preview safely".to_owned());
    }
    let mut workbook = calamine::open_workbook_auto(path).map_err(|error| error.to_string())?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or_else(|| "Workbook has no worksheets".to_owned())?
        .map_err(|error| error.to_string())?;
    let mut rows_iter = range.rows();
    let headers = rows_iter
        .next()
        .map(|row| row.iter().map(ToString::to_string).collect())
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut truncated = false;
    for row in rows_iter {
        if rows.len() >= TABLE_ROW_LIMIT {
            truncated = true;
            break;
        }
        rows.push(row.iter().map(ToString::to_string).collect());
    }
    Ok((headers, rows, truncated))
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
    } else if matches!(
        content_type,
        "text/csv"
            | "text/tab-separated-values"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-excel"
            | "application/vnd.oasis.opendocument.spreadsheet"
    ) {
        PreviewContent::Table {
            headers: Vec::new(),
            rows: Vec::new(),
            truncated: false,
        }
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
