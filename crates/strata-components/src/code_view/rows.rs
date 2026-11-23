// SPDX-License-Identifier: MIT

use std::ops::Range;
use std::sync::Arc;

/// The longest row, in bytes (Strata's `PATHOLOGICAL_TEXT_UNIT_BYTES`).
pub const CHUNK: usize = 2 * 1024;

/// Tabs stop every this many columns (Strata's `tab_width(4)`, and Rune's own stops).
pub const TAB: usize = 4;

/// One display row: bytes `start..end` of the text (no line break), part of line `line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    /// A later part of a line split at [`CHUNK`]: its gutter shows "↳", not a number.
    pub cont: bool,
}

/// The rows of a text, with where each line starts.
#[derive(Debug, Default)]
pub(crate) struct Rows {
    pub text: Arc<str>,
    rows: Vec<Row>,
    /// Each line's start in the text.
    line_starts: Vec<usize>,
    /// The most columns any row takes at least ([`floor_columns_of`]), tabs expanded.
    pub columns: usize,
    pub widest: Vec<usize>,
}

/// How many of the widest rows the view measures for its unwrapped width.
const MEASURED: usize = 6;

impl Rows {
    pub fn new(text: Arc<str>) -> Rows {
        let mut rows = Vec::new();
        let mut line_starts = Vec::new();
        let mut at = 0;
        let len = text.len();
        while at < len || line_starts.is_empty() {
            let next = text[at..].find('\n').map_or(len, |i| at + i);
            let end = if text[..next].ends_with('\r') && next > at {
                next - 1
            } else {
                next
            };
            let line = line_starts.len();
            line_starts.push(at);
            split(&text, at, end, line, &mut rows);
            at = next + 1;
            if next == len {
                break;
            }
        }
        let mut columns = 0;
        let mut by_width: Vec<(usize, usize)> = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            let s = &text[r.start..r.end];
            columns = columns.max(floor_columns_of(s));
            let n = width_of(s);
            if by_width.len() < MEASURED || n > by_width[by_width.len() - 1].0 {
                by_width.push((n, i));
                by_width.sort_by_key(|&(n, _)| std::cmp::Reverse(n));
                by_width.truncate(MEASURED);
            }
        }
        Rows {
            text,
            rows,
            line_starts,
            columns,
            widest: by_width.into_iter().map(|(_, i)| i).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn lines(&self) -> usize {
        self.line_starts.len()
    }

    pub fn row(&self, i: usize) -> Row {
        self.rows[i]
    }

    pub fn get(&self, i: usize) -> Option<Row> {
        self.rows.get(i).copied()
    }

    /// Row `i`'s bytes.
    pub fn source(&self, i: usize) -> &str {
        let r = self.rows[i];
        &self.text[r.start..r.end]
    }

    /// Where line `line` starts in the text.
    pub fn line_start(&self, line: usize) -> usize {
        self.line_starts[line]
    }

    /// Byte offset immediately after line `line`, including its line break when present.
    pub fn line_end(&self, line: usize) -> usize {
        self.line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len())
    }

    /// The logical line containing byte offset `at`, in `O(log N)` time.
    pub fn line_at_byte(&self, at: usize) -> usize {
        self.line_starts
            .partition_point(|&s| s <= at)
            .saturating_sub(1)
    }

    /// Inclusive logical-line byte range `(lo, hi)` covering lines `a..=b` (in either order).
    pub fn line_range_bytes(&self, a: usize, b: usize) -> (usize, usize) {
        let last = self.lines().saturating_sub(1);
        let lo_line = a.min(b).min(last);
        let hi_line = a.max(b).min(last);
        (self.line_start(lo_line), self.line_end(hi_line))
    }

    /// Returns `Some((lo_line, hi_line))` when `lo..hi` covers whole logical lines exactly.
    pub fn line_bounds_if_whole(&self, lo: usize, hi: usize) -> Option<(usize, usize)> {
        if lo >= hi || hi > self.text.len() || self.line_starts.is_empty() {
            return None;
        }
        let lo_line = self.line_at_byte(lo);
        let hi_line = self.line_at_byte(hi - 1);
        (self.line_start(lo_line) == lo && self.line_end(hi_line) == hi)
            .then_some((lo_line, hi_line))
    }

    /// The lines rows `rows` show part of.
    pub fn lines_of(&self, rows: Range<usize>) -> Range<usize> {
        if rows.is_empty() || self.rows.is_empty() {
            return 0..0;
        }
        let last = (rows.end - 1).min(self.rows.len() - 1);
        let first = rows.start.min(last);
        self.rows[first].line..self.rows[last].line + 1
    }

    /// The row holding byte `at` of the text: a line break belongs to the row before it.
    pub fn row_at(&self, at: usize) -> usize {
        self.rows
            .partition_point(|r| r.start <= at)
            .saturating_sub(1)
    }
}

/// Splits line `start..end` into rows, as Strata's `push_source_line_chunks` does.
fn split(text: &str, start: usize, end: usize, line: usize, rows: &mut Vec<Row>) {
    let mut at = start;
    let mut cont = false;
    while end - at > CHUNK {
        let left = end - at;
        let take = if left < CHUNK * 2 {
            left.div_ceil(2)
        } else {
            CHUNK
        };
        let mut cut = at + take;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        rows.push(Row {
            start: at,
            end: cut,
            line,
            cont,
        });
        at = cut;
        cont = true;
    }
    rows.push(Row {
        start: at,
        end,
        line,
        cont,
    });
}

fn monospaced_script(ch: char) -> bool {
    matches!(ch,
        '\u{00A0}'..='\u{052F}' // Latin-1 to Cyrillic Supplement
        | '\u{1D00}'..='\u{1DBF}' // phonetic extensions
        | '\u{1E00}'..='\u{1FFF}' // Latin Extended Additional, Greek Extended
        | '\u{2C60}'..='\u{2C7F}' // Latin Extended-C
        | '\u{2DE0}'..='\u{2DFF}' // Cyrillic Extended-A
        | '\u{A640}'..='\u{A69F}' // Cyrillic Extended-B
        | '\u{A720}'..='\u{A7FF}' // Latin Extended-D
        | '\u{AB30}'..='\u{AB6F}' // Latin Extended-E
    )
}

fn arabic(ch: char) -> bool {
    matches!(ch,
        '\u{0600}'..='\u{06FF}'
        | '\u{0750}'..='\u{077F}'
        | '\u{08A0}'..='\u{08FF}'
        | '\u{FB50}'..='\u{FDFF}'
        | '\u{FE70}'..='\u{FEFF}'
    )
}

pub(crate) fn width_of(s: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    s.chars().fold(0, |col, ch| {
        col + match ch {
            '\t' => TAB - col % TAB,
            _ if ch.is_ascii() => 1,
            _ => match ch.width() {
                None => 1,
                Some(0) => 0,
                Some(1) if monospaced_script(ch) => 1,
                Some(w) => w.max(2),
            },
        }
    })
}

pub(crate) fn floor_columns_of(s: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    s.chars().fold(0, |col, ch| {
        col + match ch {
            '\t' => TAB - col % TAB,
            _ if ch.is_ascii() => 1,
            _ if arabic(ch) || ch.width() == Some(0) => 0,
            _ => 1,
        }
    })
}

/// The columns `s` takes, each tab running to the next [`TAB`] stop.
pub(crate) fn columns_of(s: &str) -> usize {
    s.chars().fold(0, |col, ch| {
        if ch == '\t' {
            col + TAB - col % TAB
        } else {
            col + 1
        }
    })
}
