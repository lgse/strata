// SPDX-License-Identifier: MIT

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
    time::Duration,
};

use crate::sandbox::MAX_OUTPUT_BYTES;

use super::{bounded_output_with_timeout, read_limited, scale_embedded_thumbnail};

const FB2_READ_LIMIT: u64 = 16 * 1024 * 1024;
const MOBI_RECORD_LIMIT: u64 = 16 * 1024 * 1024;
const DJVU_TIMEOUT: Duration = Duration::from_secs(8);

/// Book formats that embed cover art or render a first page.
pub(super) fn render(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    render_fb2(path, size)
        .or_else(|_| render_mobi(path, size))
        .or_else(|_| render_djvu(path, size))
}

// FictionBook stores the cover as base64 in a <binary> element referenced
// from <coverpage><image href="#id"/>.
fn render_fb2(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let data = read_limited(file, FB2_READ_LIMIT).map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&data);
    let head = text.trim_start_matches(|c: char| c == '\u{feff}' || c.is_whitespace());
    if !head.starts_with("<?xml") && !head.starts_with("<FictionBook") {
        return Err("Not a FictionBook document".to_owned());
    }
    let id = coverpage_image_id(&text)?;
    let encoded = binary_element(&text, &id)?;
    scale_embedded_thumbnail(&base64_decode(encoded.as_bytes())?, size)
}

fn coverpage_image_id(text: &str) -> Result<String, String> {
    let cover = text.find("coverpage").ok_or("No coverpage")?;
    let rest = &text[cover..];
    let image = rest.find("<image").ok_or("No cover image")?;
    let tag_end = rest[image..].find('>').ok_or("Malformed cover image")?;
    let tag = &rest[image..image + tag_end];
    for quote in ['"', '\''] {
        let marker = format!("href={quote}#");
        if let Some(at) = tag.find(&marker) {
            let value = &tag[at + marker.len()..];
            let end = value.find(quote).ok_or("Malformed cover reference")?;
            return Ok(value[..end].to_owned());
        }
    }
    Err("Cover image has no reference".to_owned())
}

fn binary_element<'a>(text: &'a str, id: &str) -> Result<&'a str, String> {
    let needles = [format!("id=\"{id}\""), format!("id='{id}'")];
    let mut rest = text;
    while let Some(at) = rest.find("<binary") {
        rest = &rest[at..];
        let tag_end = rest.find('>').ok_or("Malformed binary element")?;
        if needles
            .iter()
            .any(|needle| rest[..tag_end].contains(needle))
        {
            let body = &rest[tag_end + 1..];
            let end = body.find("</binary").ok_or("Unclosed binary element")?;
            return Ok(&body[..end]);
        }
        rest = &rest[tag_end..];
    }
    Err("Cover binary not found".to_owned())
}

fn base64_decode(data: &[u8]) -> Result<Vec<u8>, String> {
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
    let mut output = Vec::with_capacity(data.len() * 3 / 4);
    let mut accumulator = 0u32;
    let mut bits = 0;
    for &byte in data {
        match value(byte) {
            Some(digit) => {
                accumulator = (accumulator << 6) | u32::from(digit);
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    output.push((accumulator >> bits) as u8);
                    accumulator &= (1 << bits) - 1;
                }
            }
            None if byte == b'=' => break,
            None => {}
        }
    }
    if output.is_empty() {
        return Err("No image data".to_owned());
    }
    Ok(output)
}

// PalmDB layout: "BOOKMOBI" signature, a record table, and a MOBI header in
// record 0 whose EXTH section points at the cover image record.
fn render_mobi(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut head = [0u8; 78];
    file.read_exact(&mut head)
        .map_err(|_| "Not a MOBI book".to_owned())?;
    if &head[60..68] != b"BOOKMOBI" {
        return Err("Not a MOBI book".to_owned());
    }
    let records = usize::from(u16::from_be_bytes(
        head[76..78].try_into().expect("2 bytes"),
    ));
    if records == 0 {
        return Err("Empty MOBI book".to_owned());
    }
    let mut table = vec![0u8; records.saturating_mul(8)];
    file.read_exact(&mut table)
        .map_err(|_| "Truncated MOBI record table".to_owned())?;
    let record_offset = |index: usize| -> Option<u64> {
        table
            .get(index * 8..index * 8 + 4)
            .map(|bytes| u64::from(u32::from_be_bytes(bytes.try_into().expect("4 bytes"))))
    };

    let record0 = record_offset(0).ok_or("Missing MOBI record 0")?;
    file.seek(SeekFrom::Start(record0))
        .map_err(|error| error.to_string())?;
    let mut mobi = [0u8; 132];
    file.read_exact(&mut mobi)
        .map_err(|_| "Truncated MOBI header".to_owned())?;
    if &mobi[16..20] != b"MOBI" {
        return Err("Not a MOBI book".to_owned());
    }
    let mobi_length = u64::from(u32::from_be_bytes(
        mobi[20..24].try_into().expect("4 bytes"),
    ));
    let first_image = u32::from_be_bytes(mobi[108..112].try_into().expect("4 bytes")) as usize;
    let exth_flags = u32::from_be_bytes(mobi[128..132].try_into().expect("4 bytes"));
    let mut cover = first_image;
    if exth_flags & 0x40 != 0 {
        file.seek(SeekFrom::Start(record0 + 16 + mobi_length))
            .map_err(|error| error.to_string())?;
        let mut exth = [0u8; 12];
        file.read_exact(&mut exth)
            .map_err(|_| "Truncated EXTH header".to_owned())?;
        if &exth[0..4] == b"EXTH" {
            let entries = u32::from_be_bytes(exth[8..12].try_into().expect("4 bytes"));
            for _ in 0..entries.min(1024) {
                let mut entry = [0u8; 8];
                file.read_exact(&mut entry)
                    .map_err(|_| "Truncated EXTH record".to_owned())?;
                let kind = u32::from_be_bytes(entry[0..4].try_into().expect("4 bytes"));
                let length = u32::from_be_bytes(entry[4..8].try_into().expect("4 bytes")) as usize;
                if length < 8 {
                    break;
                }
                // EXTH 201 carries the cover offset relative to the first image record.
                if kind == 201 && length >= 12 {
                    let mut value = [0u8; 4];
                    file.read_exact(&mut value)
                        .map_err(|_| "Truncated EXTH record".to_owned())?;
                    cover = first_image + u32::from_be_bytes(value) as usize;
                    break;
                }
                file.seek(SeekFrom::Current((length - 8) as i64))
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    if cover >= records {
        return Err("Cover record out of range".to_owned());
    }
    let start = record_offset(cover).ok_or("Missing cover record")?;
    let end = record_offset(cover + 1)
        .unwrap_or(file.metadata().map_err(|error| error.to_string())?.len());
    if end <= start || end - start > MOBI_RECORD_LIMIT {
        return Err("Cover record out of range".to_owned());
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut data = vec![0u8; (end - start) as usize];
    file.read_exact(&mut data)
        .map_err(|_| "Truncated cover record".to_owned())?;
    match &data[..data.len().min(8)] {
        [0xff, 0xd8, ..] | [0x89, b'P', b'N', b'G', ..] | [b'G', b'I', b'F', b'8', ..] => {
            scale_embedded_thumbnail(&data, size)
        }
        _ => Err("Cover record is not an image".to_owned()),
    }
}

fn render_djvu(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let mut magic = [0u8; 4];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .map_err(|_| "Not a DjVu document".to_owned())?;
    if &magic != b"AT&T" {
        return Err("Not a DjVu document".to_owned());
    }
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let page = directory.path().join("page.ppm");
    let output = bounded_output_with_timeout(
        Command::new("ddjvu")
            .arg("-page=1")
            .arg("-format=ppm")
            .arg(path)
            .arg(&page),
        MAX_OUTPUT_BYTES,
        DJVU_TIMEOUT,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "DjVu render timed out".to_owned())?;
    if !output.status.success() {
        return Err("DjVu render failed".to_owned());
    }
    let data = read_limited(
        File::open(&page).map_err(|error| error.to_string())?,
        MAX_OUTPUT_BYTES,
    )
    .map_err(|error| error.to_string())?;
    if data.is_empty() {
        return Err("DjVu produced no image".to_owned());
    }
    scale_embedded_thumbnail(&data, size)
}
