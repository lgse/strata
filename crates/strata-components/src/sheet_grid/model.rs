//! What a sheet is, as Strata hands it over, and how one cell shows.

use std::borrow::Cow;
use std::sync::Arc;

/// A cell shows at most this many bytes of its first line, then "…" (Strata's
/// `TABLE_CELL_DISPLAY_BYTES`). Copying keeps the whole text.
pub const CELL_DISPLAY_BYTES: usize = 512;

/// Strata's title cut: a column title shows this many characters, then "…".
pub const TITLE_DISPLAY_CHARS: usize = 128;

/// One worksheet, or a CSV or TSV file: Strata's `TableData` with the sheet's name. Cells are
/// the text Strata's reader gives (calamine's string conversion for workbooks), rows may be
/// ragged, and nothing here parses or recalculates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub name: Arc<str>,
    pub rows: Vec<Vec<String>>,
    /// The first row holds the column titles (Strata's rule for CSV, TSV and workbooks, when
    /// that row has a cell): it shows in the header and never sorts.
    pub header: bool,
    /// Strata's reader stopped at its value, text or column budget: the grid says so above
    /// the rows.
    pub truncated: bool,
}

impl Sheet {
    /// A sheet whose first row is the titles, unless that row is empty: then, as in Strata,
    /// there is no title row and every row sorts.
    pub fn new(name: impl Into<Arc<str>>, rows: Vec<Vec<String>>) -> Sheet {
        Sheet {
            name: name.into(),
            header: rows.first().is_some_and(|r| !r.is_empty()),
            rows,
            truncated: false,
        }
    }

    pub fn header(mut self, on: bool) -> Sheet {
        self.header = on;
        self
    }

    pub fn truncated(mut self, on: bool) -> Sheet {
        self.truncated = on;
        self
    }

    /// The widest row's cell count.
    pub fn columns(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// The rows that sort, after the title row.
    pub(crate) fn body(&self) -> &[Vec<String>] {
        &self.rows[usize::from(self.header && !self.rows.is_empty())..]
    }

    /// Column `j`'s title, if the sheet has a title row with that cell.
    pub(crate) fn title(&self, j: usize) -> Option<&str> {
        self.header
            .then(|| self.rows.first()?.get(j))
            .flatten()
            .map(String::as_str)
    }
}

/// A column's letters, as spreadsheets name them: A to Z, then AA, AB and on.
pub fn column_letters(mut j: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (j % 26) as u8);
        if j < 26 {
            break;
        }
        j = j / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII letters")
}

/// What a cell shows, and whether it was cut: its first line, at most
/// [`CELL_DISPLAY_BYTES`] of it (on a character boundary), ending in "…" when anything was
/// left out. Rows are one line high (deviation D5), so a later line shows only as the "…".
pub(crate) fn shown(text: &str) -> (Cow<'_, str>, bool) {
    let (line, more_lines) = first_line(text);
    if line.len() <= CELL_DISPLAY_BYTES && !more_lines {
        return (Cow::Borrowed(line), false);
    }
    let mut end = line.len().min(CELL_DISPLAY_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    (Cow::Owned(format!("{}…", &line[..end])), true)
}

/// What a column title shows: its first line, cut at [`TITLE_DISPLAY_CHARS`] characters,
/// marked with "…" when anything was left out.
pub(crate) fn title_shown(text: &str) -> Cow<'_, str> {
    let (line, more_lines) = first_line(text);
    match line.char_indices().nth(TITLE_DISPLAY_CHARS) {
        Some((end, _)) => Cow::Owned(format!("{}…", &line[..end])),
        None if more_lines => Cow::Owned(format!("{line}…")),
        None => Cow::Borrowed(line),
    }
}

/// A text's first line, without its line ending, and whether more lines follow. One line
/// ending at the very end starts no line of its own.
fn first_line(text: &str) -> (&str, bool) {
    let body = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix(['\n', '\r']))
        .unwrap_or(text);
    let line = body.split('\n').next().unwrap_or("");
    let line = line.strip_suffix('\r').unwrap_or(line);
    (line, line.len() < body.len())
}
