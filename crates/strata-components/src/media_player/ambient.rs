// SPDX-License-Identifier: MIT

use rune::Pixels;

/// Horizontal resolution of the sampled ambient colour grid.
pub const GRID_WIDTH: usize = 6;
/// Vertical resolution of the sampled ambient colour grid.
pub const GRID_HEIGHT: usize = 4;
/// Total number of RGB cells in the ambient grid (`GRID_WIDTH * GRID_HEIGHT`).
pub const CELLS: usize = GRID_WIDTH * GRID_HEIGHT;
/// Sub-samples per axis averaged inside each grid cell.
const TAPS: usize = 2;

/// Extra margin reserved around the video frame for the ambient halo, in points (`BAND`).
pub const BAND: f32 = 24.0;
/// Outward expansion of the blurred ambient source behind the rounded frame, in points.
pub const OUTSET: f32 = 4.0;
/// Gaussian blur standard deviation in points applied to the ambient layer.
pub const BLUR_SIGMA: f32 = 16.0;
/// Peak opacity of the ambient halo behind the video frame (`OPACITY`).
pub const OPACITY: f32 = 0.6;
/// Minimum interval in seconds between ambient texture updates (`UPDATE_INTERVAL`, 100 ms).
pub const UPDATE_INTERVAL: f64 = 0.1;
/// Exponential smoothing factor toward each newly sampled frame (`SMOOTHING`).
pub const SMOOTHING: f32 = 0.35;

/// A 6×4 row-major grid of RGB colours sampled across a video frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeGrid {
    /// Row-major RGB cells, top-left first.
    pub cells: [[u8; 3]; CELLS],
}

impl EdgeGrid {
    pub fn sample(pixels: &[u8], width: u32, height: u32) -> Option<Self> {
        let (width, height) = (width as usize, height as usize);
        let needed = width.checked_mul(height)?.checked_mul(4)?;
        if width == 0 || height == 0 || pixels.len() < needed {
            return None;
        }
        let mut cells = [[0u8; 3]; CELLS];
        for (index, cell) in cells.iter_mut().enumerate() {
            let (column, row) = (index % GRID_WIDTH, index / GRID_WIDTH);
            *cell = sample_cell(pixels, (width, height), (column, row));
        }
        Some(Self { cells })
    }
}

fn sample_cell(
    pixels: &[u8],
    (width, height): (usize, usize),
    (col, row): (usize, usize),
) -> [u8; 3] {
    let mut sum = [0u32; 3];
    for tap_y in 0..TAPS {
        let y =
            (((row * TAPS + tap_y) * 2 + 1) * height / (GRID_HEIGHT * TAPS * 2)).min(height - 1);
        for tap_x in 0..TAPS {
            let x =
                (((col * TAPS + tap_x) * 2 + 1) * width / (GRID_WIDTH * TAPS * 2)).min(width - 1);
            let offset = (y * width + x) * 4;
            sum[0] += u32::from(pixels[offset]);
            sum[1] += u32::from(pixels[offset + 1]);
            sum[2] += u32::from(pixels[offset + 2]);
        }
    }
    let samples = (TAPS * TAPS) as u32;
    [
        (sum[0] / samples) as u8,
        (sum[1] / samples) as u8,
        (sum[2] / samples) as u8,
    ]
}

/// Smoothed ambient colour state backing the persistent 6×4 [`Pixels`] handle.
#[derive(Clone, Debug)]
pub struct AmbientGlow {
    shown: [[f32; 3]; CELLS],
    pixels: Pixels,
    lit: bool,
    last_update: Option<f64>,
}

impl Default for AmbientGlow {
    fn default() -> Self {
        Self::new()
    }
}

impl AmbientGlow {
    /// Creates an unlit ambient glow with an empty persistent [`Pixels`] handle.
    pub fn new() -> Self {
        Self {
            shown: [[0.0; 3]; CELLS],
            pixels: Pixels::new(),
            lit: false,
            last_update: None,
        }
    }

    pub fn update(&mut self, grid: &EdgeGrid, now: f64) -> bool {
        if self.lit && self.is_throttled(now) {
            return false;
        }
        let fresh = !self.lit;
        let rgba = self.step_cells(grid, fresh);
        self.pixels.set(GRID_WIDTH as u32, GRID_HEIGHT as u32, rgba);
        self.lit = true;
        self.last_update = Some(now);
        true
    }

    /// Resets the ambient glow to its unlit state (on file switch, close, or error).
    pub fn clear(&mut self) {
        self.shown = [[0.0; 3]; CELLS];
        self.lit = false;
        self.last_update = None;
    }

    /// Whether the ambient glow currently holds sampled video colours.
    pub fn is_lit(&self) -> bool {
        self.lit
    }

    /// Persistent 6×4 RGBA8 [`Pixels`] handle updated in place without rebuilding the view.
    pub fn pixels(&self) -> &Pixels {
        &self.pixels
    }

    /// Current smoothed RGB cells rounded to bytes.
    pub fn shown_rgb(&self) -> [[u8; 3]; CELLS] {
        self.shown
            .map(|cell| cell.map(|channel| channel.round().clamp(0.0, 255.0) as u8))
    }

    fn is_throttled(&self, now: f64) -> bool {
        self.last_update
            .is_some_and(|last| now >= last && now - last < UPDATE_INTERVAL)
    }

    fn step_cells(&mut self, grid: &EdgeGrid, fresh: bool) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(CELLS * 4);
        for (current, target) in self.shown.iter_mut().zip(grid.cells.iter()) {
            for (channel, value) in current.iter_mut().enumerate() {
                let wanted = f32::from(target[channel]);
                *value = if fresh {
                    wanted
                } else {
                    *value + (wanted - *value) * SMOOTHING
                };
                rgba.push(value.round().clamp(0.0, 255.0) as u8);
            }
            rgba.push(255);
        }
        rgba
    }
}

#[cfg(test)]
mod tests;
