// SPDX-License-Identifier: MIT

//! Coarse colour samples from the borders of a decoded frame, for the glow
//! that bleeds around the video. A few dozen pixel reads per sample.

pub(crate) const GRID_WIDTH: usize = 6;
pub(crate) const GRID_HEIGHT: usize = 4;
pub(crate) const CELLS: usize = GRID_WIDTH * GRID_HEIGHT;
/// Samples per cell along each axis, at the cell's quarter points.
const TAPS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EdgeGrid {
    /// Row-major RGB, top-left first.
    pub(crate) cells: [[u8; 3]; CELLS],
}

/// Averages a handful of pixels per grid cell of an RGBA frame.
pub(crate) fn sample(pixels: &[u8], width: u32, height: u32) -> Option<EdgeGrid> {
    let (width, height) = (width as usize, height as usize);
    if width == 0 || height == 0 || pixels.len() < width * height * 4 {
        return None;
    }
    let mut cells = [[0; 3]; CELLS];
    for (index, cell) in cells.iter_mut().enumerate() {
        let (column, row) = (index % GRID_WIDTH, index / GRID_WIDTH);
        let mut sum = [0u32; 3];
        for tap_y in 0..TAPS {
            for tap_x in 0..TAPS {
                let x = ((column * TAPS + tap_x) * 2 + 1) * width / (GRID_WIDTH * TAPS * 2);
                let y = ((row * TAPS + tap_y) * 2 + 1) * height / (GRID_HEIGHT * TAPS * 2);
                let offset = (y.min(height - 1) * width + x.min(width - 1)) * 4;
                for (channel, total) in sum.iter_mut().enumerate() {
                    *total += u32::from(pixels[offset + channel]);
                }
            }
        }
        for (channel, total) in sum.iter().enumerate() {
            cell[channel] = (total / (TAPS * TAPS) as u32) as u8;
        }
    }
    Some(EdgeGrid { cells })
}

#[cfg(test)]
mod tests;
