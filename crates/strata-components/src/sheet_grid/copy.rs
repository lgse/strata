//! What Ctrl+C copies: tab-separated text in the displayed sort order, each cell's whole text.
//! Cells are copied as they are (no quoting), as Strata does.

use super::model::Sheet;
use super::range::CellRange;

/// The range's cells: tabs between cells, a newline between rows, none after the last, and a
/// cell a ragged row lacks copied as empty.
pub(crate) fn range_text(sheet: &Sheet, order: &[u32], r: CellRange) -> String {
    let body = sheet.body();
    let mut out = String::new();
    for (n, &src) in order[r.top..=r.bottom].iter().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let cells = &body[src as usize];
        for j in r.left..=r.right {
            if j > r.left {
                out.push('\t');
            }
            if let Some(t) = cells.get(j) {
                out.push_str(t);
            }
        }
    }
    out
}

/// Every loaded row, offscreen ones too: the title row first, then the rows in sort order,
/// each row's own cells with tabs between and a newline after (Strata's `copy_text`).
pub(crate) fn all_text(sheet: &Sheet, order: &[u32]) -> String {
    let body = sheet.body();
    let mut out = String::with_capacity(
        sheet
            .rows
            .iter()
            .map(|r| r.iter().map(|c| c.len() + 1).sum::<usize>() + 1)
            .sum(),
    );
    let mut row = |cells: &[String]| {
        for (j, t) in cells.iter().enumerate() {
            if j > 0 {
                out.push('\t');
            }
            out.push_str(t);
        }
        out.push('\n');
    };
    if sheet.header
        && let Some(titles) = sheet.rows.first()
    {
        row(titles);
    }
    for &src in order {
        row(&body[src as usize]);
    }
    out
}
