// SPDX-License-Identifier: MIT
//! Sandboxed animated WebP decoder that emits `STRRAW01` frame streams matching GIF playback.

use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use image::{
    AnimationDecoder, ColorType, DynamicImage, ImageDecoder, Rgba, codecs::webp::WebPDecoder,
    imageops, metadata::Orientation,
};

use crate::{
    media::{self, Frame, Header},
    services::{MediaPreviewSize, is_animated_webp_sample},
};

const MAX_SOURCE_PIXELS: u64 = 50_000_000;
const MAX_ANIMATION_FRAMES: u32 = 100_000;
const LOOP_GENERATION_US: u64 = 30_000_000;
const DEFAULT_ZERO_DELAY_US: u64 = 100_000;
const MAX_FRAME_CACHE_BYTES: usize = 32 * 1024 * 1024;
const HEADER_SNIFF_BYTES: usize = 30;

pub(super) fn is_animated_webp_file(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut head = [0u8; HEADER_SNIFF_BYTES];
    file.read_exact(&mut head).is_ok() && is_animated_webp_sample(&head)
}

pub(super) fn stream(
    input: &Path,
    size: MediaPreviewSize,
    start_tick: u32,
    writer: &mut impl Write,
) -> io::Result<()> {
    let info = probe(input, size, start_tick)?;
    let mut cursor = FrameCursor::open(input, &info)?;
    let first_pixels = cursor.frame_at(phase_us(start_tick, info.period_us))?;
    info.header.write(writer)?;
    write_frame(writer, start_tick, first_pixels)?;
    let mut next = start_tick + 1;
    while next < info.header.ticks() {
        let pixels = cursor.frame_at(phase_us(next, info.period_us))?;
        write_frame(writer, next, pixels)?;
        next += 1;
    }
    media::write_end(
        writer,
        next,
        info.header.duration_us.min(media::timestamp(next)),
    )?;
    writer.flush()
}

fn write_frame(writer: &mut impl Write, tick: u32, pixels: Vec<u8>) -> io::Result<()> {
    Frame {
        tick,
        pixels,
        samples: Vec::new(),
    }
    .write(writer)
}

fn phase_us(tick: u32, period_us: Option<u64>) -> u64 {
    let time_us = media::timestamp(tick);
    period_us.map_or(time_us, |period| time_us % period.max(1))
}

struct ProbeInfo {
    header: Header,
    orientation: Orientation,
    period_us: Option<u64>,
}

fn probe(path: &Path, size: MediaPreviewSize, start_tick: u32) -> io::Result<ProbeInfo> {
    let file = File::open(path)?;
    let mut decoder = WebPDecoder::new(BufReader::new(file))
        .map_err(|error| media::invalid(&format!("Unable to inspect animated WebP: {error}")))?;
    if !decoder.has_animation() {
        return Err(media::invalid("WebP image is not animated"));
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (width, height) = scaled_dimensions(decoder.dimensions(), orientation, size)?;
    let cycle_duration_us = scan_cycle_duration_us(path)?;
    let period_us = (cycle_duration_us < LOOP_GENERATION_US).then_some(cycle_duration_us);
    let duration_us = if period_us.is_some() {
        LOOP_GENERATION_US
    } else {
        cycle_duration_us.min(media::MAX_DURATION_US)
    };
    let header = Header {
        width,
        height,
        audio: false,
        duration_us,
        start_tick,
    }
    .validate(size, start_tick)?;
    Ok(ProbeInfo {
        header,
        orientation,
        period_us,
    })
}

fn orientation_swaps_axes(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

fn scaled_dimensions(
    (src_width, src_height): (u32, u32),
    orientation: Orientation,
    size: MediaPreviewSize,
) -> io::Result<(u32, u32)> {
    let pixels = u64::from(src_width).checked_mul(u64::from(src_height));
    if src_width == 0 || src_height == 0 || pixels.is_none_or(|p| p > MAX_SOURCE_PIXELS) {
        return Err(media::invalid("Unsupported source dimensions"));
    }
    let (disp_w, disp_h) = if orientation_swaps_axes(orientation) {
        (f64::from(src_height), f64::from(src_width))
    } else {
        (f64::from(src_width), f64::from(src_height))
    };
    let scale = (f64::from(size.width) / disp_w)
        .min(f64::from(size.height) / disp_h)
        .min(1.0);
    Ok((
        (disp_w * scale).floor().max(1.0) as u32,
        (disp_h * scale).floor().max(1.0) as u32,
    ))
}

fn frame_duration_us(ms: u32) -> u64 {
    if ms == 0 {
        DEFAULT_ZERO_DELAY_US
    } else {
        u64::from(ms).saturating_mul(1_000)
    }
}

/// Sums per-frame `ANMF` durations by scanning RIFF chunk headers without decoding pixels.
fn scan_cycle_duration_us(path: &Path) -> io::Result<u64> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut riff = [0u8; 12];
    reader.read_exact(&mut riff)?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WEBP" {
        return Err(media::invalid("Invalid WebP RIFF header"));
    }
    let riff_size = u64::from(u32::from_le_bytes([riff[4], riff[5], riff[6], riff[7]]));
    let end = 8_u64.saturating_add(riff_size);
    let mut position = 12_u64;
    let mut frames = 0_u32;
    let mut total_us = 0_u64;
    while position < end {
        let Some((fourcc, size)) = read_chunk_header(&mut reader)? else {
            break;
        };
        let padded = size.saturating_add(size & 1);
        if &fourcc == b"ANMF" {
            total_us = total_us
                .saturating_add(read_anmf_duration(&mut reader, size, padded, &mut frames)?)
                .min(media::MAX_DURATION_US);
        } else {
            skip_bytes(&mut reader, padded)?;
        }
        position = position.saturating_add(8).saturating_add(padded);
    }
    if frames == 0 {
        return Err(media::invalid("Animated WebP has no frames"));
    }
    Ok(total_us)
}

fn read_chunk_header(reader: &mut impl Read) -> io::Result<Option<([u8; 4], u64)>> {
    let mut header = [0u8; 8];
    match reader.read_exact(&mut header) {
        Ok(()) => {
            let fourcc = [header[0], header[1], header[2], header[3]];
            let size = u64::from(u32::from_le_bytes([
                header[4], header[5], header[6], header[7],
            ]));
            Ok(Some((fourcc, size)))
        }
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_anmf_duration(
    reader: &mut (impl Read + Seek),
    size: u64,
    padded: u64,
    frames: &mut u32,
) -> io::Result<u64> {
    if size < 24 {
        return Err(media::invalid("Invalid animated WebP frame chunk"));
    }
    *frames = frames.saturating_add(1);
    if *frames > MAX_ANIMATION_FRAMES {
        return Err(media::invalid("Animated WebP exceeds the frame limit"));
    }
    let mut anmf = [0u8; 16];
    reader.read_exact(&mut anmf)?;
    let ms = u32::from_le_bytes([anmf[12], anmf[13], anmf[14], 0]);
    skip_bytes(reader, padded - 16)?;
    Ok(frame_duration_us(ms))
}

fn skip_bytes(reader: &mut impl Seek, count: u64) -> io::Result<()> {
    let offset = i64::try_from(count).map_err(|_| media::invalid("Chunk offset overflow"))?;
    reader.seek(SeekFrom::Current(offset))?;
    Ok(())
}

fn open_frames(path: &Path) -> io::Result<image::Frames<'static>> {
    let file = File::open(path)?;
    let mut decoder = WebPDecoder::new(BufReader::new(file))
        .map_err(|error| media::invalid(&format!("Unable to open animated WebP: {error}")))?;
    if !decoder.has_animation() {
        return Err(media::invalid("WebP image is not animated"));
    }
    // Clear disposed canvas rectangles to transparent black when the stream carries alpha.
    if decoder.color_type() == ColorType::Rgba8 {
        let _ = decoder.set_background_color(Rgba([0, 0, 0, 0]));
    }
    Ok(decoder.into_frames())
}

fn prepare_frame(
    frame: image::Frame,
    orientation: Orientation,
    (width, height): (u32, u32),
) -> (u64, Vec<u8>) {
    let ms =
        u32::try_from(std::time::Duration::from(frame.delay()).as_millis()).unwrap_or(u32::MAX);
    let buffer = frame.into_buffer();
    let pixels =
        if orientation == Orientation::NoTransforms && buffer.dimensions() == (width, height) {
            buffer.into_raw()
        } else {
            let mut image = DynamicImage::ImageRgba8(buffer);
            if orientation != Orientation::NoTransforms {
                image.apply_orientation(orientation);
            }
            let rgba = image.into_rgba8();
            if rgba.dimensions() == (width, height) {
                rgba.into_raw()
            } else {
                imageops::resize(&rgba, width, height, imageops::FilterType::Triangle).into_raw()
            }
        };
    (frame_duration_us(ms), pixels)
}

struct TimedFrame {
    start_us: u64,
    end_us: u64,
    pixels: Vec<u8>,
}

struct FrameCursor {
    path: PathBuf,
    target: (u32, u32),
    orientation: Orientation,
    frames: image::Frames<'static>,
    current: Option<TimedFrame>,
    elapsed_us: u64,
    recording: Option<Vec<TimedFrame>>,
    recording_bytes: usize,
    cached: Option<Vec<TimedFrame>>,
}

impl FrameCursor {
    fn open(path: &Path, info: &ProbeInfo) -> io::Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            target: (info.header.width, info.header.height),
            orientation: info.orientation,
            frames: open_frames(path)?,
            current: None,
            elapsed_us: 0,
            recording: info.period_us.is_some().then(Vec::new),
            recording_bytes: 0,
            cached: None,
        })
    }

    fn frame_at(&mut self, phase_us: u64) -> io::Result<Vec<u8>> {
        if let Some(cached) = &self.cached {
            return Ok(select_cached(cached, phase_us));
        }
        if self
            .current
            .as_ref()
            .is_some_and(|frame| phase_us < frame.start_us)
        {
            if self.try_finalize_cache()? {
                return Ok(select_cached(
                    self.cached.as_ref().expect("finalized cache"),
                    phase_us,
                ));
            }
            self.reopen()?;
        }
        self.advance_to(phase_us)
    }

    fn record_frame(&mut self, frame: &TimedFrame) {
        let Some(recording) = &mut self.recording else {
            return;
        };
        let next_bytes = self.recording_bytes.saturating_add(frame.pixels.len());
        if next_bytes > MAX_FRAME_CACHE_BYTES {
            self.recording = None;
            self.recording_bytes = 0;
            return;
        }
        self.recording_bytes = next_bytes;
        recording.push(TimedFrame {
            start_us: frame.start_us,
            end_us: frame.end_us,
            pixels: frame.pixels.clone(),
        });
    }

    fn try_finalize_cache(&mut self) -> io::Result<bool> {
        if self.recording.is_none() {
            return Ok(false);
        }
        while let Some(next) = self.next_decoded_frame()? {
            self.record_frame(&next);
            if self.recording.is_none() {
                return Ok(false);
            }
        }
        let recorded = self.recording.take().unwrap_or_default();
        if recorded.is_empty() {
            return Ok(false);
        }
        self.cached = Some(recorded);
        Ok(true)
    }

    fn reopen(&mut self) -> io::Result<()> {
        self.frames = open_frames(&self.path)?;
        self.current = None;
        self.elapsed_us = 0;
        self.recording = None;
        self.recording_bytes = 0;
        Ok(())
    }

    fn next_decoded_frame(&mut self) -> io::Result<Option<TimedFrame>> {
        let Some(result) = self.frames.next() else {
            return Ok(None);
        };
        let frame = result
            .map_err(|error| media::invalid(&format!("Animated WebP decode failed: {error}")))?;
        let (duration_us, pixels) = prepare_frame(frame, self.orientation, self.target);
        let start_us = self.elapsed_us;
        let end_us = start_us.saturating_add(duration_us);
        self.elapsed_us = end_us;
        Ok(Some(TimedFrame {
            start_us,
            end_us,
            pixels,
        }))
    }

    fn advance_to(&mut self, phase_us: u64) -> io::Result<Vec<u8>> {
        while self
            .current
            .as_ref()
            .is_none_or(|frame| phase_us >= frame.end_us)
        {
            let Some(next) = self.next_decoded_frame()? else {
                break;
            };
            self.record_frame(&next);
            self.current = Some(next);
        }
        self.current
            .as_ref()
            .map(|frame| frame.pixels.clone())
            .ok_or_else(|| media::invalid("Animated WebP has no decodable frames"))
    }
}

fn select_cached(frames: &[TimedFrame], phase_us: u64) -> Vec<u8> {
    let index = frames.partition_point(|frame| frame.end_us <= phase_us);
    frames
        .get(index)
        .or_else(|| frames.last())
        .expect("non-empty frame cache")
        .pixels
        .clone()
}
