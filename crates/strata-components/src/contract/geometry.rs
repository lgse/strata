//! Collection geometry: where each entry sits, by index, so marquee hit-testing, keyboard moves
//! and keyboard context-menu anchors work over rows that aren't mounted.
//!
//! Every value is in points, in content coordinates: the origin is the top-left of the scroll
//! content, before scrolling. A component computes a content point from a window point as
//! window point − the viewport's window origin + the scroll offset. To anchor a popup, go the
//! other way: add the viewport's window origin and subtract the scroll offset
//! ([`Rect::offset`]).
//!
//! Don't take content coordinates from Rune's `on_drag` on the content stack. On a
//! `.virtualize` stack it reports a y rebased to the mounted rows: a press at viewport y = 40
//! with the list scrolled 28,800 pt reports y = 2034, not 28,840.

use std::ops::Range;

#[cfg(test)]
mod tests;

/// A point in content coordinates.
///
/// Used by: C1b, C1d, C1e, C2, C3, C4.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Points from the left edge.
    pub x: f32,
    /// Points from the top edge.
    pub y: f32,
}

impl Point {
    /// The point at `x`, `y`.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<(f32, f32)> for Point {
    fn from((x, y): (f32, f32)) -> Self {
        Self { x, y }
    }
}

/// A rectangle in content coordinates: its top-left corner and its size. Rune has no rect
/// type, and reports layouts as `[x, y, w, h]`, which converts both ways.
///
/// Used by: C1b, C1d, C1e, C2, C3, C4, C5 (the label rect), C21 (the keyboard anchor).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub w: f32,
    /// The height.
    pub h: f32,
}

impl Rect {
    /// The rect with top-left corner `x`, `y` and size `w` by `h`.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// The rect between two corners in any order, as a band dragged up or left from its press
    /// spans. Its size is never negative.
    pub fn from_corners(a: Point, b: Point) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    }

    /// The right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// The bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// The centre.
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Whether `p` is inside: the left and top edges are in, the right and bottom edges out.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Whether the two rects share any area.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// The same rect moved by `dx`, `dy`.
    pub fn offset(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }

    /// This rect with a non-negative size, covering the same area.
    fn normalized(self) -> Self {
        Self::from_corners(
            Point::new(self.x, self.y),
            Point::new(self.right(), self.bottom()),
        )
    }
}

impl From<[f32; 4]> for Rect {
    fn from([x, y, w, h]: [f32; 4]) -> Self {
        Self { x, y, w, h }
    }
}

impl From<Rect> for [f32; 4] {
    fn from(r: Rect) -> Self {
        [r.x, r.y, r.w, r.h]
    }
}

/// A direction to move the cursor in.
///
/// Used by: C1c, C2, C3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Towards the top.
    Up,
    /// Towards the bottom.
    Down,
    /// Towards the left.
    Left,
    /// Towards the right.
    Right,
}

/// A set of entry indices, as ascending, disjoint, non-adjacent ranges.
///
/// [`CollectionGeometry::range_in`] returns one: in a grid, a band over two columns of several
/// rows covers indices that aren't contiguous, so a single `Range` can't hold them.
///
/// Used by: C1d, C1e, C2, C3, C4, C25 (the visible range).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexRanges(Vec<Range<usize>>);

impl IndexRanges {
    /// No indices.
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Adds `range`, which must start at or after the start of every range added before it.
    /// An empty range is ignored, and one that touches or overlaps the last merges with it.
    pub fn push(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        if let Some(last) = self.0.last_mut() {
            debug_assert!(range.start >= last.start, "ranges must be pushed in order");
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                return;
            }
        }
        self.0.push(range);
    }

    /// The ranges, ascending.
    pub fn ranges(&self) -> &[Range<usize>] {
        &self.0
    }

    /// Every index, ascending.
    pub fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().flat_map(Clone::clone)
    }

    /// Whether `index` is in the set.
    pub fn contains(&self, index: usize) -> bool {
        let after = self.0.partition_point(|r| r.end <= index);
        self.0.get(after).is_some_and(|r| r.start <= index)
    }

    /// How many indices there are.
    pub fn count(&self) -> usize {
        self.0.iter().map(ExactSizeIterator::len).sum()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Range<usize>> for IndexRanges {
    fn from(range: Range<usize>) -> Self {
        let mut set = Self::new();
        set.push(range);
        set
    }
}

/// Where a collection's entries sit, by index: pure maths over every entry, mounted or not.
///
/// The list uses [`UniformRows`]. The icons grid (C3) and anything with uneven rows (C4's
/// sticky group headers) implement it themselves. Indices are those of the view's
/// `EntrySource`, and every method expects `index` below its length.
///
/// Used by: C1b (`index_at`), C1c (`neighbor`, `page_step`), C1d (`range_in`), C1e, C2, C3
/// (implements it), C4, C21 (`rect_of` the cursor, for the keyboard menu anchor).
pub trait CollectionGeometry {
    /// The entry's rect.
    fn rect_of(&self, index: usize) -> Rect;

    /// The entry under `point`, if any.
    fn index_at(&self, point: Point) -> Option<usize>;

    /// The entries whose rects share area with `rect`, which may have any sign of size. A
    /// zero-width or zero-height rect is a line or a point, and takes the entries it lies in.
    fn range_in(&self, rect: Rect) -> IndexRanges;

    /// The entry next to `index` in `direction`, or `None` at an edge. Implementations don't
    /// wrap; crossing into another collection (the next column) is the view's call.
    fn neighbor(&self, index: usize, direction: Direction) -> Option<usize>;

    /// How many entries Page Up and Page Down move by: the whole rows that fit in the viewport
    /// minus one, so a row of overlap stays in view across the jump, and at least 1. A grid
    /// moves by that many rows' worth of entries.
    ///
    /// Strata pages this way (`rows_per_page` in `src/ui/scrolling.rs`, line 512 at 35ee677ab).
    fn page_step(&self) -> usize;
}

/// Equal-height rows stacked from the top, as the list view and each Columns column draw them.
///
/// The element must draw every row at exactly `row_height`, or hit-testing over unmounted
/// rows drifts from what is drawn. Row `i` spans `[top + i·row_height, top + (i+1)·row_height)`
/// and `[0, width)`. Left and Right have no neighbours.
///
/// Used by: the tests of C1a, C1b, C1c, C1d and C1e, and C1e, C2 (one per column), C14b.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UniformRows {
    /// How many rows there are.
    pub len: usize,
    /// Each row's height.
    pub row_height: f32,
    /// Each row's width.
    pub width: f32,
    /// Space above the first row (padding, a header).
    pub top: f32,
    /// The viewport's height, for [`CollectionGeometry::page_step`].
    pub viewport_height: f32,
}

impl UniformRows {
    /// `len` rows of `row_height` by `width`, with no top inset and a zero viewport.
    pub const fn new(len: usize, row_height: f32, width: f32) -> Self {
        Self {
            len,
            row_height,
            width,
            top: 0.0,
            viewport_height: 0.0,
        }
    }

    /// The same rows, starting `inset` points down.
    pub const fn top(mut self, inset: f32) -> Self {
        self.top = inset;
        self
    }

    /// The same rows, in a viewport `height` points tall.
    pub const fn viewport_height(mut self, height: f32) -> Self {
        self.viewport_height = height;
        self
    }

    /// Whether the rows have an area to hit-test.
    fn drawable(&self) -> bool {
        self.len > 0 && self.row_height > 0.0 && self.row_height.is_finite() && self.width > 0.0
    }

    /// The row `y` falls in, as a fractional row number: may be negative or past the end.
    fn row_at(&self, y: f32) -> f64 {
        (f64::from(y) - f64::from(self.top)) / f64::from(self.row_height)
    }

    /// A fractional row number clamped to `0..=len` and truncated to a whole row. NaN (from a
    /// NaN coordinate) clamps to 0.
    fn clamp_row(&self, row: f64) -> usize {
        if row.is_nan() || row <= 0.0 {
            0
        } else if row >= self.len as f64 {
            self.len
        } else {
            // 0 < row < len here, so the cast only drops the fraction.
            row as usize
        }
    }
}

impl CollectionGeometry for UniformRows {
    fn rect_of(&self, index: usize) -> Rect {
        let y = f64::from(self.top) + index as f64 * f64::from(self.row_height);
        Rect::new(0.0, y as f32, self.width, self.row_height)
    }

    fn index_at(&self, point: Point) -> Option<usize> {
        // Written so that a NaN x, which fails every comparison, is outside too.
        let across = point.x >= 0.0 && point.x < self.width;
        if !self.drawable() || !across {
            return None;
        }
        let row = self.row_at(point.y);
        (row >= 0.0 && row < self.len as f64).then(|| self.clamp_row(row.floor()))
    }

    fn range_in(&self, rect: Rect) -> IndexRanges {
        let r = rect.normalized();
        if !self.drawable() {
            return IndexRanges::new();
        }
        let across = if r.w == 0.0 {
            r.x >= 0.0 && r.x < self.width
        } else {
            r.x < self.width && r.right() > 0.0
        };
        if !across {
            return IndexRanges::new();
        }
        if r.h == 0.0 {
            return self
                .index_at(Point::new(0.0, r.y))
                .map_or_else(IndexRanges::new, |i| (i..i + 1).into());
        }
        // Rows i with top(i) < bottom and bottom(i) > top: floor of the band's top row to the
        // ceiling of its bottom row, so a band ending exactly on a row edge leaves that row out.
        let first = self.clamp_row(self.row_at(r.y).floor());
        let end = self.clamp_row(self.row_at(r.bottom()).ceil());
        (first..end).into()
    }

    fn neighbor(&self, index: usize, direction: Direction) -> Option<usize> {
        if index >= self.len {
            return None;
        }
        match direction {
            Direction::Up => index.checked_sub(1),
            Direction::Down => (index + 1 < self.len).then_some(index + 1),
            Direction::Left | Direction::Right => None,
        }
    }

    fn page_step(&self) -> usize {
        let rows = (f64::from(self.viewport_height) / f64::from(self.row_height)).floor();
        // Also 1 for a NaN, infinite or non-positive height, where `rows` isn't a row count.
        if rows >= 1.0 && rows.is_finite() && self.row_height > 0.0 {
            (rows as usize).saturating_sub(1).max(1)
        } else {
            1
        }
    }
}
