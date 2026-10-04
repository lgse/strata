// SPDX-License-Identifier: MIT

//! Storyboard stream: a sheet header, then RGBA cells in subdivision order,
//! then an explicit end. Cells a decoder could not produce are simply absent.

use std::io::{self, Read, Write};

use super::{MAX_DURATION_US, invalid, u32_at, u64_at};

const MAGIC: &[u8; 8] = b"STRSTB01";
pub(crate) const HEADER_BYTES: usize = 28;
pub(crate) const MAX_CELL_EDGE: u32 = 192;
pub(crate) const MIN_CELL_EDGE: u32 = 32;
pub(crate) const MIN_CELLS: u32 = 8;
pub(crate) const MAX_CELLS: u32 = 48;
pub(crate) const SECONDS_PER_CELL: u64 = 2;
/// Clips shorter than this scrub well enough without a storyboard.
pub(crate) const MIN_DURATION_US: u64 = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sheet {
    pub width: u32,
    pub height: u32,
    pub count: u32,
    pub duration_us: u64,
}

impl Sheet {
    pub fn cell_bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    pub fn validate(self) -> io::Result<Self> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_CELL_EDGE
            || self.height > MAX_CELL_EDGE
            || self.count == 0
            || self.count > MAX_CELLS
            || self.duration_us == 0
            || self.duration_us >= MAX_DURATION_US
        {
            return Err(invalid("Invalid storyboard sheet"));
        }
        Ok(self)
    }

    /// The source time a cell samples: the middle of its slice of the timeline.
    pub fn cell_time_us(self, index: u32) -> u64 {
        (u64::from(index) * 2 + 1) * self.duration_us / (u64::from(self.count) * 2)
    }

    pub fn cell_at(self, time_us: u64) -> u32 {
        ((time_us.min(self.duration_us) * u64::from(self.count)) / self.duration_us)
            .min(u64::from(self.count - 1)) as u32
    }

    pub fn write(self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(MAGIC)?;
        for value in [self.width, self.height, self.count] {
            writer.write_all(&value.to_le_bytes())?;
        }
        writer.write_all(&self.duration_us.to_le_bytes())
    }

    pub fn read(reader: &mut impl Read) -> io::Result<Self> {
        let mut bytes = [0; HEADER_BYTES];
        reader.read_exact(&mut bytes)?;
        if &bytes[..8] != MAGIC {
            return Err(invalid("Unknown storyboard format"));
        }
        Self {
            width: u32_at(&bytes, 8),
            height: u32_at(&bytes, 12),
            count: u32_at(&bytes, 16),
            duration_us: u64_at(&bytes, 20),
        }
        .validate()
    }
}

pub(crate) fn cell_count(duration_us: u64) -> u32 {
    ((duration_us / 1_000_000 / SECONDS_PER_CELL) as u32).clamp(MIN_CELLS, MAX_CELLS)
}

/// Middle first, then quarters, eighths and so on, so a partial board already
/// covers the whole timeline.
pub(crate) fn subdivision_order(count: u32) -> Vec<u32> {
    let mut order = Vec::with_capacity(count as usize);
    let mut ranges = std::collections::VecDeque::from([(0, count)]);
    while let Some((low, high)) = ranges.pop_front() {
        if low >= high {
            continue;
        }
        let middle = low + (high - low) / 2;
        order.push(middle);
        ranges.push_back((low, middle));
        ranges.push_back((middle + 1, high));
    }
    order
}

pub(crate) fn write_cell(writer: &mut impl Write, index: u32, pixels: &[u8]) -> io::Result<()> {
    writer.write_all(&index.to_le_bytes())?;
    writer.write_all(&(pixels.len() as u32).to_le_bytes())?;
    writer.write_all(pixels)
}

pub(crate) fn write_end(writer: &mut impl Write, sheet: Sheet) -> io::Result<()> {
    writer.write_all(&sheet.count.to_le_bytes())?;
    writer.write_all(&0_u32.to_le_bytes())
}

pub(crate) struct CellReader {
    sheet: Sheet,
    seen: Vec<bool>,
    ended: bool,
}

impl CellReader {
    pub fn new(sheet: Sheet) -> Self {
        Self {
            sheet,
            seen: vec![false; sheet.count as usize],
            ended: false,
        }
    }

    /// The next cell, or `None` at the end record.
    pub fn read(&mut self, reader: &mut impl Read) -> io::Result<Option<(u32, Vec<u8>)>> {
        if self.ended {
            return Err(invalid("Storyboard data after its end"));
        }
        let mut bytes = [0; 8];
        reader.read_exact(&mut bytes)?;
        let index = u32_at(&bytes, 0);
        let length = u32_at(&bytes, 4) as usize;
        if index == self.sheet.count && length == 0 {
            self.ended = true;
            return Ok(None);
        }
        if index >= self.sheet.count
            || length != self.sheet.cell_bytes()
            || std::mem::replace(&mut self.seen[index as usize], true)
        {
            return Err(invalid("Invalid storyboard cell"));
        }
        let mut pixels = vec![0; length];
        reader.read_exact(&mut pixels)?;
        Ok(Some((index, pixels)))
    }
}

#[cfg(test)]
mod tests;
