// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, path::Path};

use calamine::Reader;
use serde::{Deserialize, Serialize};

use crate::sandbox::Cancellation;

use super::document::{Document, DocumentBlock, DocumentTableCell, ParsedDocument};

pub(crate) const WORKBOOK_BYTE_LIMIT: u64 = 20 * 1024 * 1024;
const TABLE_TEXT_LIMIT: usize = 4 * 1024 * 1024;
const TABLE_VALUE_LIMIT: usize = 100_000;
pub(crate) const TABLE_COLUMN_LIMIT: usize = 256;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TableData {
    pub rows: Vec<Vec<String>>,
    pub truncated: bool,
}

pub(crate) fn is_workbook(content_type: &str, name: &OsStr) -> bool {
    if Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "csv" | "tsv"))
    {
        return false;
    }
    matches!(
        content_type,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-excel"
            | "application/vnd.oasis.opendocument.spreadsheet"
    ) || Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "xlsx" | "xls" | "ods"))
}

impl TableData {
    pub(crate) fn into_document(self) -> ParsedDocument {
        let warnings = if self.truncated {
            vec!["Table preview reached its text, column, or value budget; open the file to see all data.".into()]
        } else {
            Vec::new()
        };
        ParsedDocument {
            document: Document {
                blocks: self
                    .rows
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| DocumentBlock::TableRow {
                        cells: row
                            .into_iter()
                            .map(|text| DocumentTableCell {
                                header: index == 0,
                                markup: gtk::glib::markup_escape_text(&text.replace('\0', "�"))
                                    .into(),
                            })
                            .collect(),
                    })
                    .collect(),
            },
            warnings,
        }
    }

    pub(crate) fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let data: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let mut values = 0usize;
        let mut text = 0usize;
        for row in &data.rows {
            if row.len() > TABLE_COLUMN_LIMIT {
                return Err("Table column budget exceeded".into());
            }
            values = values.saturating_add(row.len().max(1));
            for cell in row {
                text = text.saturating_add(cell.len());
            }
        }
        if values > TABLE_VALUE_LIMIT || text > TABLE_TEXT_LIMIT {
            return Err("Table preview budget exceeded".into());
        }
        Ok(data)
    }
}

pub(crate) fn parse_delimited(
    source: &str,
    delimiter: u8,
    cancellation: &Cancellation,
) -> Result<ParsedDocument, String> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(source.as_bytes());
    let mut table = TableData {
        rows: Vec::new(),
        truncated: false,
    };
    let mut values = 0;
    let mut text = 0;
    for record in reader.records() {
        if cancellation.is_cancelled() {
            return Err("Preview cancelled".into());
        }
        let record = record.map_err(|e| format!("Unable to parse table: {e}"))?;
        if !push_row(
            &mut table,
            record.iter().map(str::to_owned),
            &mut values,
            &mut text,
        ) {
            break;
        }
    }
    if table.rows.is_empty() {
        return Err("The table has no rows".into());
    }
    Ok(table.into_document())
}

fn push_row(
    table: &mut TableData,
    row: impl Iterator<Item = String>,
    values: &mut usize,
    text: &mut usize,
) -> bool {
    let mut cells = Vec::new();
    for cell in row {
        if cells.len() >= TABLE_COLUMN_LIMIT
            || *values >= TABLE_VALUE_LIMIT
            || text.saturating_add(cell.len()) > TABLE_TEXT_LIMIT
        {
            table.truncated = true;
            break;
        }
        *values += 1;
        *text += cell.len();
        cells.push(cell);
    }
    if cells.is_empty() {
        *values += 1;
    }
    if *values > TABLE_VALUE_LIMIT {
        table.truncated = true;
        return false;
    }
    table.rows.push(cells);
    !table.truncated
}

// Called only by the resource-limited sandbox helper: obtaining a calamine range
// may decompress and allocate the entire worksheet before output budgets apply.
pub(crate) fn read_workbook(path: &Path) -> Result<TableData, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > WORKBOOK_BYTE_LIMIT {
        return Err("Workbook is too large to preview safely".into());
    }
    let mut workbook = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or("Workbook has no worksheets")?
        .map_err(|e| e.to_string())?;
    let mut table = TableData {
        rows: Vec::new(),
        truncated: false,
    };
    let mut values = 0;
    let mut text = 0;
    for row in range.rows() {
        if !push_row(
            &mut table,
            row.iter().map(ToString::to_string),
            &mut values,
            &mut text,
        ) {
            break;
        }
    }
    Ok(table)
}

#[cfg(test)]
mod tests;
