// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::Pixels;

/// A sheet's shape: the cells' size in pixels, how many, and the duration they span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sheet {
    pub cell: [u32; 2],
    pub count: u32,
    pub duration: Duration,
}

impl Sheet {
    /// The cell that shows `t` (`Sheet::cell_at`).
    pub fn cell_at(&self, t: Duration) -> u32 {
        let d = self.duration.as_micros().max(1);
        let t = t.as_micros().min(d);
        ((t * u128::from(self.count)) / d).min(u128::from(self.count.saturating_sub(1))) as u32
    }

    /// The time cell `index` was taken at: the middle of its span (`Sheet::cell_time_us`).
    pub fn cell_time(&self, index: u32) -> Duration {
        let d = self.duration.as_micros();
        let us = (u128::from(index) * 2 + 1) * d / (u128::from(self.count.max(1)) * 2);
        Duration::from_micros(us as u64)
    }
}

/// A clip's storyboard: its sheet and the cells loaded so far.
#[derive(Clone, Debug)]
pub struct Storyboard {
    sheet: Sheet,
    cells: Vec<Option<Pixels>>,
}

impl Storyboard {
    /// An empty board for `sheet`.
    pub fn new(sheet: Sheet) -> Storyboard {
        Storyboard {
            sheet,
            cells: vec![None; sheet.count as usize],
        }
    }

    pub fn sheet(&self) -> Sheet {
        self.sheet
    }

    pub fn set_cell(&mut self, index: u32, pixels: Pixels) -> bool {
        match self.cells.get_mut(index as usize) {
            Some(slot @ None) => {
                *slot = Some(pixels);
                true
            }
            _ => false,
        }
    }

    /// How many cells have landed.
    pub fn loaded(&self) -> usize {
        self.cells.iter().flatten().count()
    }

    pub fn nearest(&self, t: Duration) -> Option<(u32, &Pixels)> {
        let wanted = self.sheet.cell_at(t) as usize;
        (0..self.cells.len()).find_map(|distance| {
            let before = wanted
                .checked_sub(distance)
                .and_then(|i| self.cells[i].as_ref().map(|p| (i as u32, p)));
            before.or_else(|| {
                self.cells
                    .get(wanted + distance)
                    .and_then(|c| c.as_ref())
                    .map(|p| ((wanted + distance) as u32, p))
            })
        })
    }
}
