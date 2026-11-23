// SPDX-License-Identifier: MIT

//! Bounded PCM byte ring buffer, unheard-tail rewind buffer, and in-place linear gain scaling.

use std::collections::VecDeque;

use super::SampleFormat;

/// Maximum supported linear volume factor (allows modest software boost with saturation/clamping).
pub const MAX_VOLUME: f32 = 2.0;

/// Sanitizes a volume factor: non-finite values map to silence, finite values clamp to `0.0..=MAX_VOLUME`.
pub(super) fn sanitize_volume(volume: f32) -> f32 {
    if volume.is_finite() {
        volume.clamp(0.0, MAX_VOLUME)
    } else {
        0.0
    }
}

/// Applies linear amplitude gain in place to interleaved PCM bytes.
pub(super) fn apply_gain(format: SampleFormat, bytes: &mut [u8], gain: f32) {
    if gain <= 0.0 {
        bytes.fill(0);
        return;
    }
    if (gain - 1.0).abs() < 1e-6 {
        return;
    }
    match format {
        SampleFormat::S16Le => scale_s16_le(bytes, gain),
        SampleFormat::F32Le => scale_f32_le(bytes, gain),
    }
}

fn scale_s16_le(bytes: &mut [u8], gain: f32) {
    let min = f32::from(i16::MIN);
    let max = f32::from(i16::MAX);
    for sample in bytes.as_chunks_mut::<2>().0 {
        let raw = i16::from_le_bytes(*sample);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "clamped to i16::MIN..=i16::MAX before casting"
        )]
        let scaled = (f32::from(raw) * gain).round().clamp(min, max) as i16;
        *sample = scaled.to_le_bytes();
    }
}

fn scale_f32_le(bytes: &mut [u8], gain: f32) {
    for sample in bytes.as_chunks_mut::<4>().0 {
        let raw = f32::from_le_bytes(*sample);
        let scaled = if raw.is_finite() {
            (raw * gain).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        *sample = scaled.to_le_bytes();
    }
}

/// Bounded ring buffer of interleaved PCM bytes with a small rewind prefix for pause/resume.
pub(super) struct PcmRing {
    buffer: Vec<u8>,
    capacity: usize,
    read: usize,
    len: usize,
    rewind: VecDeque<u8>,
}

impl PcmRing {
    pub(super) fn new(capacity_bytes: usize, frame_bytes: usize) -> Self {
        let frame = frame_bytes.max(1);
        let capacity = (capacity_bytes / frame).max(1) * frame;
        Self {
            buffer: vec![0; capacity],
            capacity,
            read: 0,
            len: 0,
            rewind: VecDeque::new(),
        }
    }

    pub(super) fn capacity(&self) -> usize {
        self.capacity
    }

    pub(super) fn len(&self) -> usize {
        self.len + self.rewind.len()
    }

    pub(super) fn free(&self) -> usize {
        self.capacity.saturating_sub(self.len)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len == 0 && self.rewind.is_empty()
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > self.free() {
            return false;
        }
        if bytes.is_empty() {
            return true;
        }
        let write = (self.read + self.len) % self.capacity;
        let first = (self.capacity - write).min(bytes.len());
        self.buffer[write..write + first].copy_from_slice(&bytes[..first]);
        let rest = bytes.len() - first;
        if rest > 0 {
            self.buffer[..rest].copy_from_slice(&bytes[first..]);
        }
        self.len += bytes.len();
        true
    }

    /// Prepends unplayed bytes ahead of the ring (used when pausing flushes a device buffer).
    pub(super) fn push_front(&mut self, bytes: &[u8]) {
        for &byte in bytes.iter().rev() {
            self.rewind.push_front(byte);
        }
    }

    pub(super) fn pop_into(&mut self, dst: &mut [u8], frame_bytes: usize) -> usize {
        let frame = frame_bytes.max(1);
        let max_bytes = (dst.len().min(self.len()) / frame) * frame;
        if max_bytes == 0 {
            return 0;
        }
        let mut written = 0;
        while written < max_bytes {
            let Some(byte) = self.rewind.pop_front() else {
                break;
            };
            dst[written] = byte;
            written += 1;
        }
        let from_ring = max_bytes - written;
        if from_ring > 0 {
            self.pop_ring_into(&mut dst[written..written + from_ring]);
        }
        max_bytes
    }

    fn pop_ring_into(&mut self, dst: &mut [u8]) {
        let count = dst.len();
        let first = (self.capacity - self.read).min(count);
        dst[..first].copy_from_slice(&self.buffer[self.read..self.read + first]);
        let rest = count - first;
        if rest > 0 {
            dst[first..].copy_from_slice(&self.buffer[..rest]);
        }
        self.read = (self.read + count) % self.capacity;
        self.len -= count;
    }

    pub(super) fn clear(&mut self) {
        self.read = 0;
        self.len = 0;
        self.rewind.clear();
    }
}

pub(super) struct UnheardTail {
    bytes: VecDeque<u8>,
    max_bytes: usize,
}

impl UnheardTail {
    pub(super) fn new(max_bytes: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(max_bytes.min(65_536)),
            max_bytes,
        }
    }

    pub(super) fn record(&mut self, chunk: &[u8]) {
        if self.max_bytes == 0 || chunk.is_empty() {
            return;
        }
        let keep = if chunk.len() >= self.max_bytes {
            self.bytes.clear();
            &chunk[chunk.len() - self.max_bytes..]
        } else {
            let overflow = (self.bytes.len() + chunk.len()).saturating_sub(self.max_bytes);
            if overflow > 0 {
                self.bytes.drain(..overflow);
            }
            chunk
        };
        self.bytes.extend(keep);
    }

    /// Takes up to the trailing `count` bytes (rounded down to whole frames) and clears the tail.
    pub(super) fn take_trailing(&mut self, count: usize, frame_bytes: usize) -> Vec<u8> {
        let frame = frame_bytes.max(1);
        let available = (count.min(self.bytes.len()) / frame) * frame;
        if available == 0 {
            self.bytes.clear();
            return Vec::new();
        }
        let start = self.bytes.len() - available;
        let out: Vec<u8> = self.bytes.drain(start..).collect();
        self.bytes.clear();
        out
    }

    pub(super) fn clear(&mut self) {
        self.bytes.clear();
    }
}
