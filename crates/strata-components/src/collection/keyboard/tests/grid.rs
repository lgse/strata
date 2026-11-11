//! A fake icons grid: `cols` tiles a row, as C3 lays them out, for moves by rows as laid out.

use crate::contract::{CollectionGeometry, Direction, IndexRanges, Point, Rect};

const TILE: f32 = 100.0;

/// `len` tiles, `cols` to a row, in a viewport `rows` tiles tall.
#[derive(Clone, Copy, Debug)]
pub(super) struct Grid {
    pub len: usize,
    pub cols: usize,
    pub rows: usize,
}

impl CollectionGeometry for Grid {
    fn rect_of(&self, index: usize) -> Rect {
        let (row, col) = (index / self.cols, index % self.cols);
        Rect::new(col as f32 * TILE, row as f32 * TILE, TILE, TILE)
    }

    fn index_at(&self, point: Point) -> Option<usize> {
        if point.x < 0.0 || point.y < 0.0 {
            return None;
        }
        let (col, row) = ((point.x / TILE) as usize, (point.y / TILE) as usize);
        let i = row * self.cols + col;
        (col < self.cols && i < self.len).then_some(i)
    }

    fn range_in(&self, _rect: Rect) -> IndexRanges {
        IndexRanges::new()
    }

    fn neighbor(&self, index: usize, direction: Direction) -> Option<usize> {
        if index >= self.len {
            return None;
        }
        let col = index % self.cols;
        match direction {
            Direction::Up => index.checked_sub(self.cols),
            Direction::Down => Some(index + self.cols).filter(|&i| i < self.len),
            Direction::Left => (col > 0).then(|| index - 1),
            Direction::Right => (col + 1 < self.cols && index + 1 < self.len).then(|| index + 1),
        }
    }

    /// The rows that fit minus one, times the tiles a row: Strata's Icons page.
    fn page_step(&self) -> usize {
        self.rows.saturating_sub(1).max(1) * self.cols
    }
}
