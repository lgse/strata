// SPDX-License-Identifier: MIT

//! C11b [`SheetGrid`] adapter for delimited tables (`CSV`, `TSV`) and sandboxed workbooks
//! (`XLS`, `XLSX`, `ODS`).

use std::{ffi::OsStr, path::Path};

use rune::{Cx, El};
use strata_components::{
    preview_drawer::DocumentView,
    sheet_grid::{Sheet, SheetGrid, SheetGridState},
};
use strata_fs::services::{DocumentLayout, DocumentUnit, DocumentUnitKind};

use super::labels;
use crate::App;

/// Key of the spreadsheet/table viewer inside the preview drawer.
pub(crate) const SHEET_KEY: &str = "preview-sheet";

/// Whether `entry` is a spreadsheet workbook or delimited table file (`CSV`, `TSV`, `XLS`,
/// `XLSX`, `ODS`). Tables inside Markdown, HTML, DOCX, or ODT documents stay in C11's
/// `DocumentView` (`SheetGrid` D10).
pub(super) fn is_table_file(content_type: &str, name: &OsStr) -> bool {
    if strata_preview::services::table::is_workbook(content_type, name) {
        return true;
    }
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(mime.as_str(), "text/csv" | "text/tab-separated-values") {
        return true;
    }
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "csv" | "tsv"))
}

/// Converts a single-table [`DocumentLayout`] into a [`SheetGridState`].
pub(super) fn from_layout(
    layout: &DocumentLayout,
    warnings: &[String],
    source_available: bool,
) -> Option<SheetGridState> {
    let [
        DocumentUnit {
            kind:
                DocumentUnitKind::Table {
                    list_depth: None,
                    rows,
                },
            ..
        },
    ] = layout.units.as_slice()
    else {
        return None;
    };
    let cell_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row.iter().map(|cell| cell.text.clone()).collect())
        .collect();
    let header = rows
        .first()
        .is_some_and(|row| row.iter().any(|cell| cell.header));
    let sheet = Sheet::new("", cell_rows)
        .header(header)
        .truncated(!warnings.is_empty());
    let mut state = SheetGridState::new(SHEET_KEY);
    state.set_labels(labels::sheet());
    state.set_switch(source_available.then_some(DocumentView::Rendered));
    state.set_single(sheet);
    Some(state)
}

pub(super) fn build_view(
    app: &App,
    cx: &mut Cx<App>,
    get: fn(&App) -> &SheetGridState,
    get_mut: fn(&mut App) -> &mut SheetGridState,
) -> El {
    SheetGrid::new(get, get_mut).build(app, &app.look, cx)
}
