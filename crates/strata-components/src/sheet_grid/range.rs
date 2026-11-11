//! Cells, the cursor and cell ranges, in display coordinates (rows in sort order), and the
//! sideways arithmetic that turns a point into a column.

/// A cell by its row in the current sort order and its column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellPos {
    pub row: usize,
    pub col: usize,
}

impl CellPos {
    pub fn new(row: usize, col: usize) -> CellPos {
        CellPos { row, col }
    }
}

/// A rectangle of whole cells, both ends included (deviation D2: Strata's GTK range is
/// reading-order text with character offsets in its end cells).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRange {
    pub top: usize,
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
}

impl CellRange {
    pub fn spanning(a: CellPos, b: CellPos) -> CellRange {
        CellRange {
            top: a.row.min(b.row),
            left: a.col.min(b.col),
            bottom: a.row.max(b.row),
            right: a.col.max(b.col),
        }
    }

    pub fn contains(&self, p: CellPos) -> bool {
        (self.top..=self.bottom).contains(&p.row) && (self.left..=self.right).contains(&p.col)
    }
}

/// The keyboard cursor and the end a range grows from. A range exists only while they differ:
/// a click, or a drag that stays in one cell, leaves just the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub anchor: CellPos,
    pub cursor: CellPos,
}

impl Selection {
    pub fn at(p: CellPos) -> Selection {
        Selection {
            anchor: p,
            cursor: p,
        }
    }

    pub fn range(&self) -> Option<CellRange> {
        (self.anchor != self.cursor).then(|| CellRange::spanning(self.anchor, self.cursor))
    }
}

/// Where each column starts and ends sideways, after the row-number gutter.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Columns {
    pub gutter: f32,
    /// Each column's start, then the last column's end.
    pub edges: Vec<f32>,
}

impl Columns {
    pub fn new(gutter: f32, widths: &[f32]) -> Columns {
        let mut edges = Vec::with_capacity(widths.len() + 1);
        let mut x = gutter;
        edges.push(x);
        for w in widths {
            x += w;
            edges.push(x);
        }
        Columns { gutter, edges }
    }

    pub fn count(&self) -> usize {
        self.edges.len().saturating_sub(1)
    }

    /// The content's width: the gutter and every column.
    pub fn width(&self) -> f32 {
        self.edges.last().copied().unwrap_or(self.gutter)
    }

    /// The column at content `x`, clamped to the first and last.
    pub fn at(&self, x: f32) -> usize {
        let n = self.count();
        if n == 0 {
            return 0;
        }
        // The first edge past `x`, less one: the column `x` falls in.
        let i = self.edges.partition_point(|&e| e <= x);
        i.saturating_sub(1).min(n - 1)
    }

    pub fn span(&self, j: usize) -> (f32, f32) {
        (self.edges[j], self.edges[j + 1])
    }

    /// The columns that show, at least partly, in a viewport `width` wide scrolled `offset`
    /// sideways, past the gutter that covers the start of what scrolls under it.
    pub fn showing(&self, offset: f32, width: f32) -> std::ops::Range<usize> {
        if self.count() == 0 {
            return 0..0;
        }
        self.at(offset + self.gutter)..self.at(offset + width - 0.5) + 1
    }

    /// The columns to build for a view scrolled `offset`: what shows and half a viewport's
    /// width either side, so a sideways scroll builds afresh about once half a viewport width.
    pub fn window(&self, offset: f32, width: f32) -> std::ops::Range<usize> {
        if self.count() == 0 {
            return 0..0;
        }
        let w = width.max(1.0);
        self.at((offset - w / 2.0).max(self.gutter))..self.at(offset + 1.5 * w) + 1
    }
}
