// SPDX-License-Identifier: MIT
//! Image, camera RAW and embedded thumbnail rendering through pixbuf, ImageMagick and dcraw.

use std::{
    fs,
    io::{self, Seek},
    path::Path,
    process::{Command, Stdio},
};

use gdk_pixbuf::prelude::*;

use super::{
    isobmff,
    process::{bounded_output, read_limited, successful_stdout},
};
use crate::sandbox::{FILE_SIZE_LIMIT_BYTES, MAX_OUTPUT_BYTES};

const RGBA_CHANNELS: u64 = 4;

const MAX_DECODED_PIXELS: u64 = FILE_SIZE_LIMIT_BYTES / RGBA_CHANNELS;

pub fn exceeds_decoded_frame_budget(width: i32, height: i32) -> bool {
    let width = u64::try_from(width).unwrap_or(0);
    let height = u64::try_from(height).unwrap_or(0);
    width.saturating_mul(height) > MAX_DECODED_PIXELS
}

fn positive_dimensions(width: i32, height: i32) -> Option<(i32, i32)> {
    (width > 0 && height > 0).then_some((width, height))
}

fn read_byte<R: io::Read>(reader: &mut R) -> Option<u8> {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte).ok()?;
    Some(byte[0])
}

/// Skips to the next marker after its `0xFF` fill bytes.
fn next_jpeg_marker<R: io::Read>(reader: &mut R) -> Option<u8> {
    while read_byte(reader)? != 0xFF {}
    let mut marker = read_byte(reader)?;
    while marker == 0xFF {
        marker = read_byte(reader)?;
    }
    Some(marker)
}

fn is_parameterless_jpeg_marker(marker: u8) -> bool {
    (0xD0..=0xD8).contains(&marker) || marker == 0x01
}

fn is_start_of_frame(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
}

fn read_start_of_frame<R: io::Read>(reader: &mut R) -> Option<(i32, i32)> {
    let mut sof_buf = [0u8; 5];
    reader.read_exact(&mut sof_buf).ok()?;
    let height = i32::from(u16::from_be_bytes([sof_buf[1], sof_buf[2]]));
    let width = i32::from(u16::from_be_bytes([sof_buf[3], sof_buf[4]]));
    positive_dimensions(width, height)
}

pub(super) fn read_jpeg_dimensions<R: io::Read + io::Seek>(reader: &mut R) -> Option<(i32, i32)> {
    let mut header = [0u8; 2];
    reader.read_exact(&mut header).ok()?;
    if header != [0xFF, 0xD8] {
        return None;
    }
    loop {
        let marker = next_jpeg_marker(reader)?;
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        if is_parameterless_jpeg_marker(marker) {
            continue;
        }
        let mut len_buf = [0u8; 2];
        reader.read_exact(&mut len_buf).ok()?;
        let length = u16::from_be_bytes(len_buf) as usize;
        if length < 2 {
            return None;
        }
        if is_start_of_frame(marker) {
            return read_start_of_frame(reader);
        }
        reader
            .seek(io::SeekFrom::Current((length - 2) as i64))
            .ok()?;
    }
}

pub(super) fn read_png_dimensions<R: io::Read>(reader: &mut R) -> Option<(i32, i32)> {
    let mut buf = [0u8; 24];
    reader.read_exact(&mut buf).ok()?;
    if &buf[0..8] != b"\x89PNG\r\n\x1a\n" || &buf[12..16] != b"IHDR" {
        return None;
    }
    let width = i32::try_from(u32::from_be_bytes(buf[16..20].try_into().ok()?)).ok()?;
    let height = i32::try_from(u32::from_be_bytes(buf[20..24].try_into().ok()?)).ok()?;
    positive_dimensions(width, height)
}

pub(super) fn read_gif_dimensions<R: io::Read>(reader: &mut R) -> Option<(i32, i32)> {
    let mut buf = [0u8; 10];
    reader.read_exact(&mut buf).ok()?;
    if &buf[0..6] != b"GIF87a" && &buf[0..6] != b"GIF89a" {
        return None;
    }
    let width = i32::from(u16::from_le_bytes([buf[6], buf[7]]));
    let height = i32::from(u16::from_le_bytes([buf[8], buf[9]]));
    positive_dimensions(width, height)
}

pub(super) fn image_dimensions(path: &Path) -> Option<(i32, i32)> {
    if let Ok(file) = fs::File::open(path) {
        let mut reader = io::BufReader::new(file);
        if let Some(dimensions) = read_jpeg_dimensions(&mut reader) {
            return Some(dimensions);
        }
        let _ = reader.seek(io::SeekFrom::Start(0));
        if let Some(dimensions) = read_png_dimensions(&mut reader) {
            return Some(dimensions);
        }
        let _ = reader.seek(io::SeekFrom::Start(0));
        if let Some(dimensions) = read_gif_dimensions(&mut reader) {
            return Some(dimensions);
        }
        if let Some(dimensions) = isobmff::dimensions(&mut reader) {
            return Some(dimensions);
        }
    }
    gdk_pixbuf::Pixbuf::file_info(path)
        .filter(|(_, width, height)| *width > 0 && *height > 0)
        .map(|(_, width, height)| (width, height))
}

pub(super) fn render_pixbuf(
    path: &Path,
    size: i32,
    dimensions: Option<(i32, i32)>,
) -> Result<Vec<u8>, String> {
    if let Some((width, height)) = dimensions
        && exceeds_decoded_frame_budget(width, height)
    {
        return Err("Image dimensions exceed the decoded frame budget".to_owned());
    }
    gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true)
        .map_err(|error| error.to_string())?
        .save_to_bufferv("png", &[("compression", "1")])
        .map_err(|error| error.to_string())
}

pub(super) fn render_raw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    // Preserve small sources so the preview can bound upscaling by their native dimensions.
    let info = image_dimensions(path);
    if let Some((width, height)) =
        info.and_then(|(width, height)| positive_dimensions(width, height))
    {
        if exceeds_decoded_frame_budget(width, height) {
            return render_oversized(path, size)
                .ok_or_else(|| "Image dimensions exceed the decoded frame budget".to_owned());
        }
        return render_pixbuf(path, size.min(width.max(height)), info)
            .or_else(|_| render_imagemagick(path, size))
            .or_else(|_| render_dcraw(path, size));
    }
    render_imagemagick(path, size).or_else(|_| render_dcraw(path, size))
}

pub(super) fn render_camera_raw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let dimensions = image_dimensions(path);
    if let Some((width, height)) = dimensions
        && exceeds_decoded_frame_budget(width, height)
    {
        return read_exif_thumbnail(path, size)
            .or_else(|| render_dcraw(path, size).ok())
            .ok_or_else(|| "Image dimensions exceed the decoded frame budget".to_owned());
    }
    // Prefer the camera JPEG so ImageMagick does not demosaic.
    render_dcraw(path, size)
        .or_else(|_| render_pixbuf(path, size, dimensions))
        .or_else(|_| render_imagemagick(path, size))
}

pub(super) fn render_camera_raw_preview(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let dimensions = image_dimensions(path);
    let pixbuf_sees_thumbnail = dimensions.is_some_and(|(width, height)| width.max(height) < size);
    if !pixbuf_sees_thumbnail {
        return render_camera_raw(path, size);
    }
    render_dcraw(path, size)
        .or_else(|_| render_imagemagick(path, size))
        .or_else(|_| render_pixbuf(path, size, dimensions))
}

pub(super) fn render_oversized(path: &Path, size: i32) -> Option<Vec<u8>> {
    let thumbnail = read_exif_thumbnail(path, size);
    let covers_size = thumbnail
        .as_deref()
        .and_then(|png| read_png_dimensions(&mut io::Cursor::new(png)))
        .is_some_and(|(width, height)| width.max(height) >= size);
    if covers_size {
        return thumbnail;
    }
    render_reduced_jpeg(path, size).or(thumbnail)
}

fn render_reduced_jpeg(path: &Path, size: i32) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    read_jpeg_dimensions(&mut io::BufReader::new(file))?;
    render_imagemagick(path, size).ok()
}

pub(super) fn read_exif_thumbnail(path: &Path, size: i32) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut reader = io::BufReader::new(file);
    let exif = exif::Reader::new()
        .continue_on_error(true)
        .read_from_container(&mut reader)
        .or_else(|error| error.distill_partial_result(|_| {}))
        .ok()?;
    let offset = exif
        .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)
        .and_then(|field| field.value.get_uint(0))? as usize;
    let len = exif
        .get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)
        .and_then(|field| field.value.get_uint(0))? as usize;
    let end = offset.checked_add(len)?;
    let data = exif.buf().get(offset..end)?;
    scale_embedded_thumbnail(data, size).ok()
}

pub(super) fn render_imagemagick(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    for executable in ["magick", "convert"] {
        let output = bounded_output(
            Command::new(executable)
                .args(["-limit", "area"])
                .arg(MAX_DECODED_PIXELS.to_string())
                .args(["-limit", "disk", "0", "-define"])
                .arg(format!("jpeg:size={size}x{size}"))
                .arg(path)
                .args(["-auto-orient", "-thumbnail"])
                .arg(format!("{size}x{size}>"))
                .arg("png:-"),
            MAX_OUTPUT_BYTES,
        );
        if let Some(stdout) = successful_stdout(output) {
            return Ok(stdout);
        }
    }
    Err("No RAW image renderer succeeded".to_owned())
}

// LibRaw's dcraw_emu does not support `-e`; `-c` is a threshold, not stdout.
pub(super) fn render_dcraw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let classic = bounded_output(
        Command::new("dcraw").args(["-e", "-c"]).arg(path),
        MAX_OUTPUT_BYTES,
    );
    if let Some(png) =
        successful_stdout(classic).and_then(|stdout| scale_embedded_thumbnail(&stdout, size).ok())
    {
        return Ok(png);
    }
    render_simple_dcraw(path, size)
}

pub(super) fn render_simple_dcraw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::symlink;

    // Writes `<file>.thumb.jpg` next to the input, which is a read-only bind.
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let staging = directory.path().join("raw-thumb");
    symlink(path, &staging).map_err(|error| error.to_string())?;
    let status = Command::new("simple_dcraw")
        .arg("-e")
        .arg(&staging)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("simple_dcraw failed".to_owned());
    }
    for thumb in ["raw-thumb.thumb.jpg", "raw-thumb.thumb.ppm"] {
        let Ok(file) = fs::File::open(directory.path().join(thumb)) else {
            continue;
        };
        let Ok(data) = read_limited(file, MAX_OUTPUT_BYTES) else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        if let Ok(png) = scale_embedded_thumbnail(&data, size) {
            return Ok(png);
        }
    }
    Err("simple_dcraw produced no thumbnail".to_owned())
}

pub(super) fn scale_embedded_thumbnail(data: &[u8], size: i32) -> Result<Vec<u8>, String> {
    let dimensions = read_jpeg_dimensions(&mut io::Cursor::new(data))
        .or_else(|| read_png_dimensions(&mut io::Cursor::new(data)))
        .or_else(|| read_gif_dimensions(&mut io::Cursor::new(data)));
    if dimensions.is_some_and(|(width, height)| exceeds_decoded_frame_budget(width, height)) {
        return Err("Embedded thumbnail exceeds the decoded frame budget".to_owned());
    }
    scale_embedded_thumbnail_pixbuf(data, size).or_else(|_| render_imagemagick_bytes(data, size))
}

fn scale_embedded_thumbnail_pixbuf(data: &[u8], size: i32) -> Result<Vec<u8>, String> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader
        .write(data)
        .and_then(|()| loader.close())
        .map_err(|error| error.to_string())?;
    let pixbuf = loader
        .pixbuf()
        .ok_or_else(|| "Unable to decode embedded thumbnail".to_owned())?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let width = pixbuf.width().max(1);
    let height = pixbuf.height().max(1);
    let scale = (f64::from(size) / f64::from(width))
        .min(f64::from(size) / f64::from(height))
        .min(1.0);
    let target_width = (f64::from(width) * scale).round().max(1.0) as i32;
    let target_height = (f64::from(height) * scale).round().max(1.0) as i32;
    let pixbuf = if target_width == width && target_height == height {
        pixbuf
    } else {
        pixbuf
            .scale_simple(
                target_width,
                target_height,
                gdk_pixbuf::InterpType::Bilinear,
            )
            .ok_or_else(|| "Unable to scale embedded thumbnail".to_owned())?
    };
    pixbuf
        .save_to_bufferv("png", &[("compression", "1")])
        .map_err(|error| error.to_string())
}

fn render_imagemagick_bytes(data: &[u8], size: i32) -> Result<Vec<u8>, String> {
    use std::io::Write;
    if data.len() as u64 > MAX_OUTPUT_BYTES {
        return Err("Embedded thumbnail exceeds the input budget".to_owned());
    }
    let mut input = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
    input.write_all(data).map_err(|error| error.to_string())?;
    render_imagemagick(input.path(), size)
}
