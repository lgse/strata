// SPDX-License-Identifier: MIT

//! Animated WebP detection and timing, read from the RIFF container without decoding.

use std::io::{self, Read, Seek, SeekFrom};

#[cfg(test)]
mod tests;

const ANIMATION_FLAG: u8 = 0x02;
const VP8X_FLAGS_OFFSET: usize = 20;
/// The RIFF header and the VP8X chunk, which ends with the canvas size.
const HEADER_BYTES: usize = 30;
/// The frame header before an ANMF chunk's image data: offsets, size, duration and flags.
const ANMF_HEADER_BYTES: u32 = 16;
const ANMF_DURATION_OFFSET: usize = 12;

/// FFmpeg's `loop` filter holds at most this many frames.
pub const MAX_FRAMES: u32 = 32_767;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebpAnimation {
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub frames: u32,
    pub duration_us: u64,
}

pub fn is_animated_webp(sample: &[u8]) -> bool {
    sample.len() > VP8X_FLAGS_OFFSET
        && sample.starts_with(b"RIFF")
        && &sample[8..16] == b"WEBPVP8X"
        && sample[VP8X_FLAGS_OFFSET] & ANIMATION_FLAG != 0
}

/// Walks chunk headers only. Anything malformed, still, timeless or too long to
/// loop yields `None`.
pub fn read_webp_animation(reader: &mut (impl Read + Seek)) -> io::Result<Option<WebpAnimation>> {
    let Some((mut animation, riff_end)) = read_header(reader)? else {
        return Ok(None);
    };
    let mut position = HEADER_BYTES as u64;
    while position + 8 <= riff_end {
        let Some(chunk) = read_chunk(reader, position)? else {
            return Ok(None);
        };
        if chunk
            .frame_ms
            .is_some_and(|duration_ms| !animation.add_frame(duration_ms))
        {
            return Ok(None);
        }
        position = chunk.end;
    }
    // A final odd-sized chunk may lack its padding byte.
    let complete = position <= riff_end + 1;
    Ok((complete && animation.duration_us > 0).then_some(animation))
}

impl WebpAnimation {
    /// `false` once the animation outgrows the `loop` filter.
    fn add_frame(&mut self, duration_ms: u32) -> bool {
        self.frames += 1;
        self.duration_us += u64::from(duration_ms) * 1_000;
        self.frames <= MAX_FRAMES
    }
}

/// The empty animation and the end of its RIFF payload, which must lie within the file.
fn read_header(reader: &mut (impl Read + Seek)) -> io::Result<Option<(WebpAnimation, u64)>> {
    let mut header = [0; HEADER_BYTES];
    if read_full(reader, &mut header)? < header.len() || !is_animated_webp(&header) {
        return Ok(None);
    }
    let riff_end = u64::from(le_u32(&header[4..8])) + 8;
    if riff_end > reader.seek(SeekFrom::End(0))? {
        return Ok(None);
    }
    let animation = WebpAnimation {
        canvas_width: u24(&header[24..27]) + 1,
        canvas_height: u24(&header[27..30]) + 1,
        frames: 0,
        duration_us: 0,
    };
    Ok(Some((animation, riff_end)))
}

struct Chunk {
    end: u64,
    frame_ms: Option<u32>,
}

/// `None` for a chunk, or an ANMF frame header, cut short by the end of the file.
fn read_chunk(reader: &mut (impl Read + Seek), position: u64) -> io::Result<Option<Chunk>> {
    reader.seek(SeekFrom::Start(position))?;
    let mut bytes = [0; 8 + ANMF_HEADER_BYTES as usize];
    let read = read_full(reader, &mut bytes)?;
    if read < 8 {
        return Ok(None);
    }
    let size = le_u32(&bytes[4..8]);
    let is_frame = &bytes[..4] == b"ANMF";
    let has_frame_header = size >= ANMF_HEADER_BYTES && read == bytes.len();
    if is_frame && !has_frame_header {
        return Ok(None);
    }
    let duration = 8 + ANMF_DURATION_OFFSET;
    Ok(Some(Chunk {
        end: position + 8 + u64::from(size) + u64::from(size % 2),
        frame_ms: is_frame.then(|| u24(&bytes[duration..duration + 3])),
    }))
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("4 bytes"))
}

fn u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | u32::from(bytes[1]) << 8 | u32::from(bytes[2]) << 16
}

fn read_full(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}
