// SPDX-License-Identifier: MIT

//! Animated WebP detection and timing, read from the RIFF container without decoding.

use std::io::{self, Read, Seek, SeekFrom};

#[cfg(test)]
mod tests;

const ANIMATION_FLAG: u8 = 0x02;
const VP8X_FLAGS_OFFSET: usize = 20;
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

/// Whether the leading bytes are an extended WebP header that declares an animation.
pub fn is_animated_webp(sample: &[u8]) -> bool {
    sample.len() > VP8X_FLAGS_OFFSET
        && sample.starts_with(b"RIFF")
        && &sample[8..16] == b"WEBPVP8X"
        && sample[VP8X_FLAGS_OFFSET] & ANIMATION_FLAG != 0
}

/// Reads the canvas and the summed frame durations by walking chunk headers only.
/// Anything malformed, still, timeless or too long to loop yields `None`.
pub fn read_webp_animation(reader: &mut (impl Read + Seek)) -> io::Result<Option<WebpAnimation>> {
    let mut header = [0; 30];
    if read_full(reader, &mut header)? < header.len() || !is_animated_webp(&header) {
        return Ok(None);
    }
    let mut animation = WebpAnimation {
        canvas_width: u24(&header[24..27]) + 1,
        canvas_height: u24(&header[27..30]) + 1,
        frames: 0,
        duration_us: 0,
    };
    let riff_end = u64::from(u32::from_le_bytes(
        header[4..8].try_into().expect("4 bytes"),
    )) + 8;
    if riff_end > reader.seek(SeekFrom::End(0))? {
        return Ok(None);
    }
    let mut position = 30;
    while position + 8 <= riff_end {
        reader.seek(SeekFrom::Start(position))?;
        let mut chunk = [0; 8 + ANMF_HEADER_BYTES as usize];
        let read = read_full(reader, &mut chunk)?;
        if read < 8 {
            return Ok(None);
        }
        let size = u32::from_le_bytes(chunk[4..8].try_into().expect("4 bytes"));
        if &chunk[..4] == b"ANMF" {
            if size < ANMF_HEADER_BYTES || read < chunk.len() {
                return Ok(None);
            }
            animation.frames += 1;
            if animation.frames > MAX_FRAMES {
                return Ok(None);
            }
            let duration_ms = u24(&chunk[8 + ANMF_DURATION_OFFSET..8 + ANMF_DURATION_OFFSET + 3]);
            animation.duration_us += u64::from(duration_ms) * 1_000;
        }
        position += 8 + u64::from(size) + u64::from(size % 2);
    }
    if position > riff_end + 1 || animation.frames == 0 || animation.duration_us == 0 {
        return Ok(None);
    }
    Ok(Some(animation))
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
