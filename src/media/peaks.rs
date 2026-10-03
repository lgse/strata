// SPDX-License-Identifier: MIT

//! Waveform overview stream: a header, then in-order runs of bucket levels.

use std::io::{self, Read, Write};

use super::{MAX_DURATION_US, invalid, u32_at, u64_at};

pub(crate) const BUCKETS: u32 = 1024;
pub(crate) const SAMPLE_RATE: u32 = 8_000;
const MAGIC: &[u8; 8] = b"STRPEAK1";
const MAX_RUN: u32 = 256;

pub(crate) fn write_header(writer: &mut impl Write, duration_us: u64) -> io::Result<()> {
    writer.write_all(MAGIC)?;
    writer.write_all(&BUCKETS.to_le_bytes())?;
    writer.write_all(&duration_us.to_le_bytes())
}

pub(crate) fn read_header(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 20];
    reader.read_exact(&mut bytes)?;
    let duration_us = u64_at(&bytes, 12);
    if &bytes[..8] != MAGIC
        || u32_at(&bytes, 8) != BUCKETS
        || duration_us == 0
        || duration_us >= MAX_DURATION_US
    {
        return Err(invalid("Invalid waveform header"));
    }
    Ok(duration_us)
}

fn write_run(writer: &mut impl Write, start: u32, levels: &[u8]) -> io::Result<()> {
    writer.write_all(&start.to_le_bytes())?;
    writer.write_all(&(levels.len() as u32).to_le_bytes())?;
    writer.write_all(levels)
}

/// Bucket level: RMS with square-root companding so quiet passages keep detail.
pub(crate) fn level(rms: f64) -> u8 {
    (rms.clamp(0.0, 1.0).sqrt() * 255.0).round() as u8
}

pub(crate) fn rms(level: u8) -> f32 {
    (f32::from(level) / 255.0).powi(2)
}

/// Accepts 16-bit little-endian mono PCM at `SAMPLE_RATE`.
pub(crate) struct Accumulator {
    samples_per_bucket: f64,
    consumed: u64,
    bucket: u32,
    squares: f64,
    count: u64,
    flushed: u32,
    pending: Vec<u8>,
}

impl Accumulator {
    pub(crate) fn new(duration_us: u64) -> Self {
        let samples = duration_us as f64 * f64::from(SAMPLE_RATE) / 1_000_000.0;
        Self {
            samples_per_bucket: (samples / f64::from(BUCKETS)).max(1.0),
            consumed: 0,
            bucket: 0,
            squares: 0.0,
            count: 0,
            flushed: 0,
            pending: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, pcm: &[u8], writer: &mut impl Write) -> io::Result<()> {
        for sample in pcm.as_chunks::<2>().0 {
            let value = f64::from(i16::from_le_bytes([sample[0], sample[1]])) / 32_768.0;
            let bucket = ((self.consumed as f64 / self.samples_per_bucket) as u32).min(BUCKETS - 1);
            if bucket != self.bucket {
                self.close_bucket();
                self.bucket = bucket;
            }
            self.squares += value * value;
            self.count += 1;
            self.consumed += 1;
        }
        if self.pending.len() >= 16 {
            self.flush(writer)?;
        }
        Ok(())
    }

    fn close_bucket(&mut self) {
        let rms = if self.count == 0 {
            0.0
        } else {
            (self.squares / self.count as f64).sqrt()
        };
        self.pending.push(level(rms));
        self.squares = 0.0;
        self.count = 0;
    }

    fn flush(&mut self, writer: &mut impl Write) -> io::Result<()> {
        for run in self.pending.chunks(MAX_RUN as usize) {
            write_run(writer, self.flushed, run)?;
            self.flushed += run.len() as u32;
        }
        self.pending.clear();
        writer.flush()
    }

    pub(crate) fn finish(mut self, writer: &mut impl Write) -> io::Result<()> {
        if self.count > 0 {
            self.close_bucket();
        }
        self.flush(writer)?;
        write_run(writer, self.flushed, &[])
    }
}

#[derive(Default)]
pub(crate) struct RunReader {
    next: u32,
    ended: bool,
}

impl RunReader {
    pub(crate) fn read(&mut self, reader: &mut impl Read) -> io::Result<Option<(u32, Vec<u8>)>> {
        if self.ended {
            return Err(invalid("Waveform data after its end"));
        }
        let mut bytes = [0; 8];
        reader.read_exact(&mut bytes)?;
        let start = u32_at(&bytes, 0);
        let count = u32_at(&bytes, 4);
        if start != self.next || count > MAX_RUN || count > BUCKETS - start {
            return Err(invalid("Invalid waveform run"));
        }
        if count == 0 {
            self.ended = true;
            return Ok(None);
        }
        let mut levels = vec![0; count as usize];
        reader.read_exact(&mut levels)?;
        self.next += count;
        Ok(Some((start, levels)))
    }
}

#[cfg(test)]
mod tests;
