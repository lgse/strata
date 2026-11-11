//! Where tiles sit: the grid's [`Lines`] rule and its [`CollectionGeometry`], pure maths over
//! every entry, mounted or not.
//!
//! A row holds N tiles, N from the viewport width and the narrowest cell, at most the column
//! cap; the row's width less the grid's side padding is shared equally, as GTK's GridView
//! shares it. Tile `i` is in row `i / N`, column `i % N`.

use crate::collection::element::Lines;
use crate::contract::{CollectionGeometry, Direction, IndexRanges, Point, Rect};

use super::metrics::TileMetrics;

#[cfg(test)]
mod tests;

/// The grid's packing rule for the collection element (C1e): tiles a line from the width.
///
/// Used by: C3, C20d.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridLines {
    metrics: TileMetrics,
}

impl GridLines {
    /// The rule for tiles of `metrics`.
    pub fn new(metrics: TileMetrics) -> Self {
        Self { metrics }
    }
}

impl Lines for GridLines {
    fn per_line(&self, width: f32) -> usize {
        self.metrics.per_line(width)
    }

    fn inset(&self) -> f32 {
        self.metrics.grid_pad
    }

    fn geometry(
        &self,
        len: usize,
        row_height: f32,
        width: f32,
        viewport_height: f32,
    ) -> Box<dyn CollectionGeometry> {
        let per = self.per_line(width);
        Box::new(IconsGeometry {
            len,
            per,
            cell_w: self.metrics.cell_width(width, per),
            row_h: row_height,
            pad: self.metrics.grid_pad,
            viewport_h: viewport_height,
        })
    }
}

/// `len` tiles, `per` to a row of cells `cell_w` by `row_h`, starting `pad` from the left.
///
/// - **Up** moves a row up. **Down** moves a row down, or to the last tile when the row below
///   is a shorter last row with no tile under this one, as GTK's GridView does
///   (`gtk_grid_view_move_focus_along`); from the last row there is none.
/// - **Left** and **Right** stay in the row: C1c's `wrap_rows` crosses a row's end, as Strata's
///   Icons view does.
/// - **Page** is the rows that fit minus one, times the tiles a row.
///
/// Used by: C3 (through [`GridLines`]), C1c, C1d, C21.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconsGeometry {
    /// How many tiles there are.
    pub len: usize,
    /// Tiles a row: at least 1.
    pub per: usize,
    /// A cell's width.
    pub cell_w: f32,
    /// A row's height.
    pub row_h: f32,
    /// The space left of the first column (and right of the last).
    pub pad: f32,
    /// The viewport's height, for paging.
    pub viewport_h: f32,
}

impl IconsGeometry {
    fn per(&self) -> usize {
        self.per.max(1)
    }

    fn rows(&self) -> usize {
        self.len.div_ceil(self.per())
    }

    fn drawable(&self) -> bool {
        self.len > 0
            && self.row_h > 0.0
            && self.row_h.is_finite()
            && self.cell_w > 0.0
            && self.cell_w.is_finite()
    }

    /// The fractional column `x` falls in.
    fn col_at(&self, x: f32) -> f64 {
        (f64::from(x) - f64::from(self.pad)) / f64::from(self.cell_w)
    }

    /// The fractional row `y` falls in.
    fn row_at(&self, y: f32) -> f64 {
        f64::from(y) / f64::from(self.row_h)
    }
}

/// `v` clamped to `0..=max` and cut to a whole number; NaN is 0.
fn clamp(v: f64, max: usize) -> usize {
    if v.is_nan() || v <= 0.0 {
        0
    } else if v >= max as f64 {
        max
    } else {
        v as usize
    }
}

impl CollectionGeometry for IconsGeometry {
    fn rect_of(&self, index: usize) -> Rect {
        let per = self.per();
        let x = f64::from(self.pad) + (index % per) as f64 * f64::from(self.cell_w);
        let y = (index / per) as f64 * f64::from(self.row_h);
        Rect::new(x as f32, y as f32, self.cell_w, self.row_h)
    }

    fn index_at(&self, point: Point) -> Option<usize> {
        if !self.drawable() {
            return None;
        }
        let (c, r) = (self.col_at(point.x), self.row_at(point.y));
        if !(c >= 0.0 && c < self.per() as f64 && r >= 0.0 && r < self.rows() as f64) {
            return None;
        }
        let i = r.floor() as usize * self.per() + c.floor() as usize;
        (i < self.len).then_some(i)
    }

    fn range_in(&self, rect: Rect) -> IndexRanges {
        let r = Rect::from_corners(
            Point::new(rect.x, rect.y),
            Point::new(rect.right(), rect.bottom()),
        );
        let mut out = IndexRanges::new();
        if !self.drawable() {
            return out;
        }
        let (per, rows) = (self.per(), self.rows());
        // A zero-width or zero-height band takes the cells it lies in.
        let (c0, c1) = if r.w == 0.0 {
            let c = self.col_at(r.x);
            if !(c >= 0.0 && c < per as f64) {
                return out;
            }
            (c as usize, c as usize + 1)
        } else {
            (
                clamp(self.col_at(r.x).floor(), per),
                clamp(self.col_at(r.right()).ceil(), per),
            )
        };
        let (r0, r1) = if r.h == 0.0 {
            let row = self.row_at(r.y);
            if !(row >= 0.0 && row < rows as f64) {
                return out;
            }
            (row as usize, row as usize + 1)
        } else {
            (
                clamp(self.row_at(r.y).floor(), rows),
                clamp(self.row_at(r.bottom()).ceil(), rows),
            )
        };
        if c0 >= c1 {
            return out;
        }
        for row in r0..r1 {
            let start = row * per + c0;
            let end = (row * per + c1).min(self.len);
            if start < end {
                out.push(start..end);
            }
        }
        out
    }

    fn neighbor(&self, index: usize, direction: Direction) -> Option<usize> {
        if index >= self.len {
            return None;
        }
        let per = self.per();
        let col = index % per;
        match direction {
            Direction::Up => index.checked_sub(per),
            Direction::Down => {
                ((self.len - 1) / per > index / per).then(|| (index + per).min(self.len - 1))
            }
            Direction::Left => (col > 0).then(|| index - 1),
            Direction::Right => (col + 1 < per && index + 1 < self.len).then(|| index + 1),
        }
    }

    fn page_step(&self) -> usize {
        let rows = (f64::from(self.viewport_h) / f64::from(self.row_h)).floor();
        let rows = if rows >= 1.0 && rows.is_finite() && self.row_h > 0.0 {
            (rows as usize).saturating_sub(1).max(1)
        } else {
            1
        };
        rows * self.per()
    }
}
