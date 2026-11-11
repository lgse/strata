//! Storyboard frames: small cells across the clip, shown over the timeline's pointer and
//! over the frame while a seek runs.
//!
//! Strata's helper decodes a sheet of 8 to 48 cells, about one per 2 s, and streams them
//! middle first, then quarters, eighths and so on, so a partial board already covers the
//! timeline (`src/media/storyboard.rs`). The host loads it on
//! [`MediaIntent::StoryboardWanted`](super::MediaIntent::StoryboardWanted) and hands each cell
//! over as it lands ([`Storyboard::set_cell`]); a cell not loaded yet shows its nearest loaded
//! neighbour.

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

    /// Cell `index` landed. False for an index outside the sheet or one already loaded (the
    /// helper sends each once).
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

    /// The loaded cell nearest the one that shows `t`, earlier first on a tie
    /// (`Storyboard::nearest`), with its index.
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
