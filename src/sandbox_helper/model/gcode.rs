// SPDX-License-Identifier: MIT

use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
};

use resvg::tiny_skia::{IntSize, Pixmap};

use crate::services::{ModelPreviewStage, model_preview::MAX_GCODE_HEADER_BYTES};

use super::embedded::{
    MAX_THUMBNAIL_BYTES, MAX_THUMBNAIL_CANDIDATES, MAX_TOTAL_THUMBNAIL_BYTES,
    MAX_TOTAL_THUMBNAIL_PIXELS, scale_pixmap,
};

const NO_THUMBNAIL: &str = crate::services::model_preview::NO_GCODE_THUMBNAIL;
const MAX_BASE64_CHARS: usize = (MAX_THUMBNAIL_BYTES as usize) * 4 / 3 + 4;
const MAX_LINE_BYTES: usize = MAX_BASE64_CHARS + 32;
/// Header cap, plus one thumbnail and the base64 lines that carry it.
const ABSOLUTE_READ_BYTES: u64 = MAX_GCODE_HEADER_BYTES + MAX_THUMBNAIL_BYTES + 65_536;
const MAX_BGCODE_BLOCKS: usize = 64;

const BGCODE_MAGIC: &[u8; 4] = b"GCDE";
const GCODE_BLOCK: u16 = 1;
const THUMBNAIL_BLOCK: u16 = 5;

struct Candidate {
    pixels: u64,
    bytes: Vec<u8>,
}

enum Block {
    Keep { pixels: u64, encoded: String },
    Drop,
}

pub(super) fn thumbnail(
    input: &Path,
    width: u32,
    height: u32,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<Vec<u8>, String> {
    let mut found = collect(input)?;
    if found.is_empty() {
        return Err(NO_THUMBNAIL.to_owned());
    }
    found.sort_by_key(|candidate| std::cmp::Reverse(candidate.pixels));
    progress(ModelPreviewStage::Thumbnail);
    for candidate in found {
        if let Ok(png) = raster_png(&candidate.bytes, width, height) {
            progress(ModelPreviewStage::Finishing);
            return Ok(png);
        }
    }
    Err(NO_THUMBNAIL.to_owned())
}

fn collect(input: &Path) -> Result<Vec<Candidate>, String> {
    let mut file = File::open(input).map_err(|error| error.to_string())?;
    let mut magic = [0; 4];
    let read = file.read(&mut magic).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    if read == 4
        && &magic == BGCODE_MAGIC
        && let Some(candidates) = binary_candidates(&mut file)?
    {
        return Ok(candidates);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    text_candidates(&mut file)
}

/// Parameters sit in front of the sized payload and are not included in
/// `uncompressed_size` or `compressed_size`. Thumbnail blocks are uncompressed.
fn binary_candidates(file: &mut File) -> Result<Option<Vec<Candidate>>, String> {
    let mut magic = [0; 4];
    if read_exact(file, &mut magic)?.is_none() || &magic != BGCODE_MAGIC {
        return Ok(None);
    }
    let Some(version) = read_u32(file)? else {
        return Ok(None);
    };
    if version != 1 {
        return Ok(None);
    }
    let checksum_len = match read_u16(file)? {
        Some(0) => 0,
        Some(1) => 4,
        _ => return Ok(None),
    };
    let mut candidates = Vec::new();
    for _ in 0..MAX_BGCODE_BLOCKS {
        let Some(kind) = read_u16(file)? else {
            break;
        };
        let Some(compression) = read_u16(file)? else {
            break;
        };
        let Some(uncompressed) = read_u32(file)? else {
            break;
        };
        let stored = if compression == 0 {
            u64::from(uncompressed)
        } else {
            let Some(compressed) = read_u32(file)? else {
                break;
            };
            u64::from(compressed)
        };
        if kind == GCODE_BLOCK {
            break;
        }
        let parameters: u64 = match kind {
            THUMBNAIL_BLOCK => 6,
            0 | 2 | 3 | 4 => 2,
            _ => break,
        };
        if kind == THUMBNAIL_BLOCK && compression == 0 {
            let mut params = [0; 6];
            if read_exact(file, &mut params)?.is_none() {
                break;
            }
            if u64::from(uncompressed) > MAX_THUMBNAIL_BYTES {
                skip(file, u64::from(uncompressed))?;
            } else {
                let mut data = vec![0; uncompressed as usize];
                if read_exact(file, &mut data)?.is_none() {
                    break;
                }
                let format = u16::from_le_bytes([params[0], params[1]]);
                let width = u16::from_le_bytes([params[2], params[3]]);
                let height = u16::from_le_bytes([params[4], params[5]]);
                let pixels = u64::from(width).saturating_mul(u64::from(height));
                if format <= 2 && pixels > 0 && pixels <= MAX_TOTAL_THUMBNAIL_PIXELS {
                    consider(&mut candidates, data, pixels);
                }
            }
        } else {
            skip(file, parameters + stored)?;
        }
        skip(file, checksum_len)?;
    }
    Ok(Some(candidates))
}

fn text_candidates(file: &mut File) -> Result<Vec<Candidate>, String> {
    let mut reader = BufReader::new(file);
    let mut candidates = Vec::new();
    let mut block = None;
    let mut consumed = 0u64;
    loop {
        let start = consumed;
        let Some(line) = next_line(&mut reader, ABSOLUTE_READ_BYTES.saturating_sub(consumed))?
        else {
            break;
        };
        consumed += line.bytes;
        if block.is_none() && start >= MAX_GCODE_HEADER_BYTES {
            break;
        }
        if !line.usable {
            block = None;
            if start >= MAX_GCODE_HEADER_BYTES || consumed >= ABSOLUTE_READ_BYTES {
                break;
            }
            continue;
        }
        let text = line.text.trim();
        if text.is_empty() || text.starts_with('(') {
            continue;
        }
        let Some(body) = text.strip_prefix(';') else {
            break;
        };
        let body = body.trim();
        if let Some(marker) = marker(body) {
            match marker {
                Marker::Begin { pixels, keep } => {
                    block = Some(if keep {
                        Block::Keep {
                            pixels,
                            encoded: String::new(),
                        }
                    } else {
                        Block::Drop
                    });
                }
                Marker::End => {
                    if let Some(Block::Keep { pixels, encoded }) = block.take()
                        && let Some(bytes) = decode_base64(&encoded)
                    {
                        consider(&mut candidates, bytes, pixels);
                    }
                }
            }
            continue;
        }
        match &mut block {
            Some(Block::Keep { encoded, .. }) => {
                if body.is_empty() {
                    continue;
                }
                if !push_base64(encoded, body) {
                    block = Some(Block::Drop);
                }
            }
            Some(Block::Drop) | None => {}
        }
    }
    if let Some(Block::Keep { pixels, encoded }) = block
        && let Some(bytes) = decode_base64(&encoded)
    {
        consider(&mut candidates, bytes, pixels);
    }
    Ok(candidates)
}

struct TextLine {
    text: String,
    bytes: u64,
    usable: bool,
}

fn next_line(reader: &mut impl std::io::BufRead, budget: u64) -> Result<Option<TextLine>, String> {
    if budget == 0 {
        return Ok(None);
    }
    let mut text = Vec::new();
    let mut bytes = 0u64;
    let mut usable = true;
    loop {
        let (piece, consume_len, finished, budget_spent) = {
            let available = reader.fill_buf().map_err(|error| error.to_string())?;
            if available.is_empty() {
                (Vec::new(), 0, true, false)
            } else {
                let room = usize::try_from(budget.saturating_sub(bytes)).unwrap_or(0);
                if room == 0 {
                    (Vec::new(), 0, true, true)
                } else {
                    let take = available.len().min(room);
                    let end = available[..take].iter().position(|byte| *byte == b'\n');
                    let copy_end = end.unwrap_or(take);
                    let consume_len = end.map(|index| index + 1).unwrap_or(take);
                    (
                        available[..copy_end].to_vec(),
                        consume_len,
                        end.is_some(),
                        false,
                    )
                }
            }
        };
        if consume_len == 0 {
            if bytes == 0 {
                return Ok(None);
            }
            if budget_spent {
                usable = false;
            }
            break;
        }
        if usable && text.len() + piece.len() <= MAX_LINE_BYTES {
            text.extend_from_slice(&piece);
        } else {
            usable = false;
        }
        bytes += consume_len as u64;
        reader.consume(consume_len);
        if finished {
            break;
        }
    }
    if text.last() == Some(&b'\r') {
        text.pop();
    }
    let text = String::from_utf8(text).ok();
    let usable = usable
        && text
            .as_ref()
            .is_some_and(|text| !text.as_bytes().contains(&0));
    Ok(Some(TextLine {
        text: text.unwrap_or_default(),
        bytes,
        usable,
    }))
}

enum Marker {
    Begin { pixels: u64, keep: bool },
    End,
}

fn marker(body: &str) -> Option<Marker> {
    let mut parts = body.split_whitespace();
    let kind = parts.next()?;
    let action = parts.next()?.to_ascii_lowercase();
    let lowered = kind.to_ascii_lowercase();
    let suffix = lowered.strip_prefix("thumbnail")?;
    if !matches!(suffix, "" | "_png" | "_jpg" | "_jpeg" | "_qoi") {
        return None;
    }
    match action.as_str() {
        "end" => Some(Marker::End),
        "begin" => {
            let (width, height) = parts.next()?.split_once('x')?;
            let width: u32 = width.parse().ok()?;
            let height: u32 = height.parse().ok()?;
            let pixels = u64::from(width).saturating_mul(u64::from(height));
            let declared = parts.next().and_then(|length| length.parse::<usize>().ok());
            let keep = pixels > 0
                && pixels <= MAX_TOTAL_THUMBNAIL_PIXELS
                && declared.is_none_or(|length| length <= MAX_BASE64_CHARS);
            Some(Marker::Begin { pixels, keep })
        }
        _ => None,
    }
}

fn push_base64(encoded: &mut String, body: &str) -> bool {
    if encoded.len().saturating_add(body.len()) > MAX_BASE64_CHARS {
        return false;
    }
    let alphabet = body.bytes().all(|byte| {
        matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' | b'='
        )
    });
    if !alphabet {
        return false;
    }
    encoded.push_str(body);
    true
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    if input.is_empty() || input.len() % 4 == 1 {
        return None;
    }
    let mut padded = input.as_bytes().to_vec();
    while !padded.len().is_multiple_of(4) {
        padded.push(b'=');
    }
    let mut output = Vec::with_capacity(padded.len() / 4 * 3);
    for (index, chunk) in padded.as_chunks::<4>().0.iter().enumerate() {
        let last = index + 1 == padded.len() / 4;
        let (a, b, c, d) = (chunk[0], chunk[1], chunk[2], chunk[3]);
        if (c == b'=' && d != b'=') || ((c == b'=' || d == b'=') && !last) {
            return None;
        }
        let (Some(a), Some(b)) = (value(a), value(b)) else {
            return None;
        };
        output.push((a << 2) | (b >> 4));
        if c != b'=' {
            let c = value(c)?;
            output.push((b << 4) | (c >> 2));
            if d != b'=' {
                let d = value(d)?;
                output.push((c << 6) | d);
            }
        }
        if output.len() as u64 > MAX_THUMBNAIL_BYTES {
            return None;
        }
    }
    Some(output)
}

fn consider(candidates: &mut Vec<Candidate>, bytes: Vec<u8>, declared: u64) {
    if bytes.is_empty() || bytes.len() as u64 > MAX_THUMBNAIL_BYTES {
        return;
    }
    let pixels = match image_pixels(&bytes) {
        Some(pixels) if pixels == 0 || pixels > MAX_TOTAL_THUMBNAIL_PIXELS => return,
        Some(pixels) => pixels,
        None if declared == 0 || declared > MAX_TOTAL_THUMBNAIL_PIXELS => return,
        None => declared,
    };
    let mut discard = Vec::new();
    let mut total: usize = candidates
        .iter()
        .map(|candidate| candidate.bytes.len())
        .sum();
    let mut count = candidates.len();
    while count >= MAX_THUMBNAIL_CANDIDATES
        || total + bytes.len() > MAX_TOTAL_THUMBNAIL_BYTES as usize
    {
        let Some((index, candidate)) = candidates
            .iter()
            .enumerate()
            .filter(|(index, _)| !discard.contains(index))
            .min_by_key(|(_, candidate)| candidate.pixels)
        else {
            return;
        };
        if candidate.pixels >= pixels {
            return;
        }
        discard.push(index);
        total -= candidate.bytes.len();
        count -= 1;
    }
    discard.sort_unstable();
    for index in discard.into_iter().rev() {
        candidates.remove(index);
    }
    candidates.push(Candidate { pixels, bytes });
}

fn image_pixels(bytes: &[u8]) -> Option<u64> {
    if let Some((width, height)) = crate::sandbox::png_dimensions(bytes) {
        return Some(u64::from(width).saturating_mul(u64::from(height)));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        && let Ok((width, height)) =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg)
                .into_dimensions()
    {
        return Some(u64::from(width).saturating_mul(u64::from(height)));
    }
    if bytes.starts_with(b"qoif") && bytes.len() >= 12 {
        let width = u32::from_be_bytes(bytes[4..8].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[8..12].try_into().ok()?);
        return Some(u64::from(width).saturating_mul(u64::from(height)));
    }
    None
}

fn raster_png(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let image = if crate::sandbox::png_dimensions(bytes).is_some() {
        Pixmap::decode_png(bytes).map_err(|_| "Invalid model thumbnail")?
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        jpeg_pixmap(bytes)?
    } else if bytes.starts_with(b"qoif") {
        qoi_pixmap(bytes)?
    } else {
        return Err("Invalid model thumbnail".into());
    };
    let pixels = u64::from(image.width()).saturating_mul(u64::from(image.height()));
    if pixels == 0 || pixels > MAX_TOTAL_THUMBNAIL_PIXELS {
        return Err("Invalid model thumbnail".into());
    }
    scale_pixmap(&image, width, height)
}

fn jpeg_pixmap(bytes: &[u8]) -> Result<Pixmap, String> {
    let invalid = || "Invalid model thumbnail".to_owned();
    let (width, height) =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg)
            .into_dimensions()
            .map_err(|_| invalid())?;
    if over_limit(width, height) {
        return Err(invalid());
    }
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .map_err(|_| invalid())?;
    pixmap_from_rgba(image.to_rgba8()).ok_or_else(invalid)
}

fn qoi_pixmap(data: &[u8]) -> Result<Pixmap, String> {
    let invalid = || "Invalid model thumbnail".to_owned();
    if data.len() < 14 || &data[..4] != b"qoif" {
        return Err(invalid());
    }
    let width = u32::from_be_bytes(data[4..8].try_into().map_err(|_| invalid())?);
    let height = u32::from_be_bytes(data[8..12].try_into().map_err(|_| invalid())?);
    if over_limit(width, height) {
        return Err(invalid());
    }
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(invalid)?;
    let mut pixels = vec![0; count * 4];
    let mut index = [[0u8; 4]; 64];
    let mut pixel = [0u8, 0, 0, 255];
    let mut run = 0usize;
    let mut cursor = 14usize;
    for offset in 0..count {
        if run > 0 {
            run -= 1;
        } else if cursor < data.len() {
            let opcode = data[cursor];
            cursor += 1;
            if opcode == 0xFE {
                if cursor + 3 > data.len() {
                    return Err(invalid());
                }
                pixel[0] = data[cursor];
                pixel[1] = data[cursor + 1];
                pixel[2] = data[cursor + 2];
                cursor += 3;
            } else if opcode == 0xFF {
                if cursor + 4 > data.len() {
                    return Err(invalid());
                }
                pixel.copy_from_slice(&data[cursor..cursor + 4]);
                cursor += 4;
            } else if opcode & 0xC0 == 0x00 {
                pixel = index[(opcode & 0x3F) as usize];
            } else if opcode & 0xC0 == 0x40 {
                pixel[0] = pixel[0].wrapping_add((opcode >> 4) & 0x03).wrapping_sub(2);
                pixel[1] = pixel[1].wrapping_add((opcode >> 2) & 0x03).wrapping_sub(2);
                pixel[2] = pixel[2].wrapping_add(opcode & 0x03).wrapping_sub(2);
            } else if opcode & 0xC0 == 0x80 {
                if cursor >= data.len() {
                    return Err(invalid());
                }
                let next = data[cursor];
                cursor += 1;
                let green = (opcode & 0x3F).wrapping_sub(32);
                pixel[0] = pixel[0]
                    .wrapping_add(green)
                    .wrapping_add((next >> 4).wrapping_sub(8));
                pixel[1] = pixel[1].wrapping_add(green);
                pixel[2] = pixel[2]
                    .wrapping_add(green)
                    .wrapping_add((next & 0x0F).wrapping_sub(8));
            } else {
                run = (opcode & 0x3F) as usize;
            }
            let hash = pixel[0] as usize * 3
                + pixel[1] as usize * 5
                + pixel[2] as usize * 7
                + pixel[3] as usize * 11;
            index[hash % 64] = pixel;
        } else {
            return Err(invalid());
        }
        let start = offset * 4;
        pixels[start..start + 4].copy_from_slice(&pixel);
    }
    // Pixmap stores premultiplied pixels. PNG decoding does that itself.
    premultiply(&mut pixels);
    let size = IntSize::from_wh(width, height).ok_or_else(invalid)?;
    Pixmap::from_vec(pixels, size).ok_or_else(invalid)
}

fn pixmap_from_rgba(image: image::RgbaImage) -> Option<Pixmap> {
    let (width, height) = image.dimensions();
    if over_limit(width, height) {
        return None;
    }
    let mut pixels = image.into_raw();
    premultiply(&mut pixels);
    Pixmap::from_vec(pixels, IntSize::from_wh(width, height)?)
}

fn premultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

fn over_limit(width: u32, height: u32) -> bool {
    width == 0
        || height == 0
        || u64::from(width).saturating_mul(u64::from(height)) > MAX_TOTAL_THUMBNAIL_PIXELS
}

fn read_u16(file: &mut File) -> Result<Option<u16>, String> {
    let mut bytes = [0; 2];
    Ok(read_exact(file, &mut bytes)?.map(|_| u16::from_le_bytes(bytes)))
}

fn read_u32(file: &mut File) -> Result<Option<u32>, String> {
    let mut bytes = [0; 4];
    Ok(read_exact(file, &mut bytes)?.map(|_| u32::from_le_bytes(bytes)))
}

fn read_exact(file: &mut File, bytes: &mut [u8]) -> Result<Option<()>, String> {
    match file.read_exact(bytes) {
        Ok(()) => Ok(Some(())),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn skip(file: &mut File, bytes: u64) -> Result<(), String> {
    let bytes = i64::try_from(bytes).map_err(|_| NO_THUMBNAIL.to_owned())?;
    file.seek(SeekFrom::Current(bytes))
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests;
