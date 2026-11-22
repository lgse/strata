// SPDX-License-Identifier: MIT

//! Interactive per-table state for embedded Markdown/document tables inside
//! [`DocumentView`](super::DocumentView): column width measurement and manual resizing,
//! multi-key column header sorting with a three-state cycle, display-order copy formatting,
//! and C24 horizontal overlay scroll state.

pub(crate) mod render;
pub(crate) mod sort;

use std::cell::OnceCell;

use rune::TextStyle;
use rune_kit::SortDir;

use super::model::Cell;
use crate::scroll::Bar;
use sort::ColumnKeys;

/// Narrowest manual drag width for a column (matching GTK's `table_view.rs`).
pub const MIN_WIDTH: f32 = 48.0;
/// Widest manual drag width for a column.
pub const MAX_WIDTH: f32 = 4000.0;

const SAMPLE_ROWS: usize = 64;
const SAMPLE_LINES: usize = 8;
const SAMPLE_CHARS: usize = 128;
const WANT_MIN: f32 = 72.0;
const WANT_MAX: f32 = 640.0;
const SHARE_FLOOR: f32 = 160.0;
pub(crate) const CELL_PAD_X: f32 = 10.0;
pub(crate) const CELL_PAD_Y: f32 = 5.0;

/// Fonts used to measure and draw a table's body and header cells.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TableFonts {
    pub(crate) body: TextStyle,
    pub(crate) header: TextStyle,
}

/// Per-table interactive state keyed by table unit index inside [`Content`](super::state::Content).
#[derive(Debug)]
pub(crate) struct TableState {
    pub(crate) header: bool,
    pub(crate) columns: usize,
    body_count: usize,
    sort: Vec<(usize, SortDir)>,
    order: Vec<u32>,
    keys: Vec<OnceCell<ColumnKeys>>,
    wanted: Option<(u32, Vec<f32>)>,
    fitted: Vec<f32>,
    drawn: Vec<f32>,
    manual: Option<Vec<f32>>,
    pub(crate) bar: Bar,
}

impl TableState {
    pub(crate) fn new(rows: &[Vec<Cell>]) -> Self {
        let header = has_header(rows);
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let body_count = rows.len().saturating_sub(usize::from(header));
        let order = (0..body_count as u32).collect();
        let keys = (0..columns).map(|_| OnceCell::new()).collect();
        Self {
            header,
            columns,
            body_count,
            sort: Vec::new(),
            order,
            keys,
            wanted: None,
            fitted: Vec::new(),
            drawn: Vec::new(),
            manual: None,
            bar: Bar::new(),
        }
    }

    pub(crate) fn is_sorted(&self) -> bool {
        !self.sort.is_empty()
    }

    pub(crate) fn sort(&self) -> &[(usize, SortDir)] {
        &self.sort
    }

    pub(crate) fn order(&self) -> &[u32] {
        &self.order
    }

    /// Source row indices of all displayed body rows.
    pub(crate) fn body_source_rows(&self) -> Vec<usize> {
        let offset = usize::from(self.header);
        self.order.iter().map(|&r| offset + r as usize).collect()
    }

    pub(crate) fn click_header(&mut self, rows: &[Vec<Cell>], column: usize, shift: bool) {
        if column >= self.columns {
            return;
        }
        let next = if shift {
            sort::shift_clicked(&self.sort, column)
        } else {
            sort::clicked(&self.sort, column)
        };
        self.apply_sort(rows, next);
    }

    fn apply_sort(&mut self, rows: &[Vec<Cell>], spec: Vec<(usize, SortDir)>) {
        if self.sort == spec {
            return;
        }
        let prev = std::mem::replace(&mut self.sort, spec);
        if self.sort.is_empty() || self.body_count <= 1 {
            self.order = (0..self.body_count as u32).collect();
            return;
        }
        let body = &rows[usize::from(self.header)..];
        if is_primary_flip(&prev, &self.sort) {
            let primary = self.column_keys(body, self.sort[0].0);
            self.order = sort::flipped(&self.order, primary);
            return;
        }
        for &(col, _) in &self.sort {
            self.column_keys(body, col);
        }
        let specs: Vec<(&ColumnKeys, SortDir)> = self
            .sort
            .iter()
            .map(|&(col, dir)| (self.keys[col].get().expect("initialized above"), dir))
            .collect();
        self.order = sort::order(self.body_count, &specs);
    }

    fn column_keys(&self, body: &[Vec<Cell>], column: usize) -> &ColumnKeys {
        self.keys[column].get_or_init(|| ColumnKeys::new(body, column))
    }

    /// Resolves current column widths for layout: caches measured content widths per font size,
    /// fits them to `available` width until the user resizes a column manually.
    pub(crate) fn resolve_widths(
        &mut self,
        rows: &[Vec<Cell>],
        fonts: TableFonts,
        available: Option<f32>,
    ) -> Vec<f32> {
        if self.columns == 0 {
            return Vec::new();
        }
        let key = fonts.body.size.to_bits();
        if self.wanted.as_ref().is_none_or(|(s, _)| *s != key) {
            let measured = measure_columns(rows, self.columns, self.header, fonts);
            self.wanted = Some((key, measured));
        }
        let wanted = &self.wanted.as_ref().expect("measured above").1;
        let fitted = match available {
            Some(avail) if avail > 0.0 => fit_widths(wanted, avail),
            _ => wanted.clone(),
        };
        let active = self.manual.clone().unwrap_or_else(|| fitted.clone());
        self.fitted = fitted;
        self.drawn = active.clone();
        active
    }

    pub(crate) fn column_width(&self, column: usize) -> Option<f32> {
        self.manual
            .as_ref()
            .and_then(|m| m.get(column).copied())
            .or_else(|| self.drawn.get(column).copied())
    }

    pub(crate) fn column_widths(&self) -> Option<Vec<f32>> {
        if let Some(m) = &self.manual {
            return Some(m.clone());
        }
        (!self.drawn.is_empty()).then(|| self.drawn.clone())
    }

    pub(crate) fn set_column_width(&mut self, column: usize, width: f32) {
        if column >= self.columns {
            return;
        }
        let widths = self.manual.get_or_insert_with(|| {
            if self.drawn.len() == self.columns {
                self.drawn.clone()
            } else {
                vec![WANT_MIN; self.columns]
            }
        });
        widths[column] = width.clamp(MIN_WIDTH, MAX_WIDTH);
        self.drawn = widths.clone();
    }

    /// Double-click auto-fit: snaps column `column` back to its content-measured width.
    pub(crate) fn fit_column(&mut self, column: usize) {
        if column >= self.columns {
            return;
        }
        let target = self
            .wanted
            .as_ref()
            .and_then(|(_, w)| w.get(column).copied())
            .or_else(|| self.fitted.get(column).copied())
            .unwrap_or(WANT_MIN);
        self.set_column_width(column, target);
    }

    /// Formats the table's rows as tab-separated lines in the currently displayed (sorted) order.
    pub(crate) fn copy_text(&self, rows: &[Vec<Cell>]) -> String {
        let mut out = String::new();
        if self.header
            && let Some(head) = rows.first()
        {
            append_row(&mut out, head);
        }
        let offset = usize::from(self.header);
        for &idx in &self.order {
            if let Some(row) = rows.get(offset + idx as usize) {
                append_row(&mut out, row);
            }
        }
        out
    }
}

/// Whether row 0 is a header row (non-empty and every cell marked `header`), matching GTK.
pub(crate) fn has_header(rows: &[Vec<Cell>]) -> bool {
    rows.first()
        .is_some_and(|row| !row.is_empty() && row.iter().all(|c| c.header))
}

fn is_primary_flip(prev: &[(usize, SortDir)], next: &[(usize, SortDir)]) -> bool {
    prev.len() == next.len()
        && !next.is_empty()
        && prev[0].0 == next[0].0
        && prev[0].1 != next[0].1
        && prev[1..] == next[1..]
}

fn append_row(out: &mut String, row: &[Cell]) {
    for (c, cell) in row.iter().enumerate() {
        if c > 0 {
            out.push('\t');
        }
        out.push_str(&cell.text);
    }
    out.push('\n');
}

/// Measures each column's desired width from up to the first 64 rows, matching GTK's
/// `measure_columns` (`72..=640`, extra room for padding and the header sort indicator).
fn measure_columns(
    rows: &[Vec<Cell>],
    columns: usize,
    header: bool,
    fonts: TableFonts,
) -> Vec<f32> {
    let mut out = vec![WANT_MIN; columns];
    for (r, row) in rows.iter().take(SAMPLE_ROWS).enumerate() {
        let is_title = r == 0 && header;
        let font = if is_title { fonts.header } else { fonts.body };
        // Horizontal padding on both sides plus sort-indicator clearance in headers.
        let extra = 2.0 * CELL_PAD_X + if is_title { 22.0 } else { 12.0 };
        for (c, cell) in row.iter().enumerate() {
            if c >= columns {
                break;
            }
            out[c] = out[c].max(measure_cell(&cell.text, font, extra));
        }
    }
    out
}

fn measure_cell(text: &str, font: TextStyle, extra: f32) -> f32 {
    let mut widest = 0.0f32;
    for line in text.lines().take(SAMPLE_LINES) {
        let sample: String = line.chars().take(SAMPLE_CHARS).collect();
        let w = (rune::text::advance(&sample, font).ceil() + extra).min(WANT_MAX);
        widest = widest.max(w);
    }
    widest
}

/// Distributes `available` width across columns (matching GTK `table_view.rs` & `sheet_grid`):
/// each column keeps at least `min(wanted, 160)` and shares remaining width proportionally.
/// If the floors exceed `available`, columns keep their floors and the table scrolls sideways.
pub(crate) fn fit_widths(wanted: &[f32], available: f32) -> Vec<f32> {
    let floors: Vec<f32> = wanted.iter().map(|w| w.min(SHARE_FLOOR)).collect();
    let extra = (available - floors.iter().sum::<f32>()).max(0.0);
    let weights: Vec<f32> = wanted
        .iter()
        .zip(&floors)
        .map(|(w, f)| (w - f).max(1.0))
        .collect();
    let total: f32 = weights.iter().sum::<f32>().max(1.0);
    let mut left = extra;
    let n = wanted.len();
    floors
        .iter()
        .zip(&weights)
        .enumerate()
        .map(|(c, (f, w))| {
            let share = if c + 1 == n {
                left
            } else {
                (extra * w / total).floor()
            };
            left -= share;
            f + share
        })
        .collect()
}
