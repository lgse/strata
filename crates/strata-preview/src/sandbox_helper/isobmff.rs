// SPDX-License-Identifier: MIT

use std::io::{Read, Seek, SeekFrom};

const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_BOXES: usize = 4096;

struct BoxHeader {
    kind: [u8; 4],
    payload_size: u64,
}

fn read_box(reader: &mut impl Read, remaining: u64) -> Option<BoxHeader> {
    if remaining < 8 {
        return None;
    }
    let mut header = [0; 8];
    reader.read_exact(&mut header).ok()?;
    let size = u32::from_be_bytes(header[..4].try_into().ok()?);
    let (size, header_size) = match size {
        0 => (remaining, 8),
        1 => {
            if remaining < 16 {
                return None;
            }
            let mut extended = [0; 8];
            reader.read_exact(&mut extended).ok()?;
            (u64::from_be_bytes(extended), 16)
        }
        size => (u64::from(size), 8),
    };
    if size > remaining {
        return None;
    }
    Some(BoxHeader {
        kind: header[4..].try_into().ok()?,
        payload_size: size.checked_sub(header_size)?,
    })
}

pub(super) fn dimensions(reader: &mut (impl Read + Seek)) -> Option<(i32, i32)> {
    let end = reader.seek(SeekFrom::End(0)).ok()?;
    reader.rewind().ok()?;
    let header = read_box(reader, end)?;
    if &header.kind != b"ftyp" || header.payload_size < 8 {
        return None;
    }
    reader
        .seek(SeekFrom::Current(i64::try_from(header.payload_size).ok()?))
        .ok()?;
    for _ in 0..MAX_BOXES {
        let remaining = end.checked_sub(reader.stream_position().ok()?)?;
        let header = read_box(reader, remaining)?;
        if &header.kind == b"meta" {
            if header.payload_size > MAX_METADATA_BYTES {
                return None;
            }
            let mut metadata = vec![0; usize::try_from(header.payload_size).ok()?];
            reader.read_exact(&mut metadata).ok()?;
            return metadata_dimensions(&metadata);
        }
        // Compressed pixels can precede the metadata; never read an mdat payload.
        reader
            .seek(SeekFrom::Current(i64::try_from(header.payload_size).ok()?))
            .ok()?;
    }
    None
}

struct ImageBox<'a> {
    kind: [u8; 4],
    data: &'a [u8],
}

fn children(mut data: &[u8]) -> Option<Vec<ImageBox<'_>>> {
    let mut boxes = Vec::new();
    while !data.is_empty() {
        if boxes.len() == MAX_BOXES {
            return None;
        }
        let remaining = data.len() as u64;
        let header = read_box(&mut data, remaining)?;
        let (payload, rest) = data.split_at_checked(usize::try_from(header.payload_size).ok()?)?;
        boxes.push(ImageBox {
            kind: header.kind,
            data: payload,
        });
        data = rest;
    }
    Some(boxes)
}

fn number(reader: &mut impl Read, size: usize) -> Option<u32> {
    let mut bytes = [0; 4];
    reader
        .read_exact(bytes.get_mut(4_usize.checked_sub(size)?..)?)
        .ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn metadata_dimensions(mut metadata: &[u8]) -> Option<(i32, i32)> {
    if number(&mut metadata, 4)? != 0 {
        return None;
    }
    let metadata = children(metadata)?;
    let mut primary = metadata.iter().find(|item| &item.kind == b"pitm")?.data;
    let primary_version = number(&mut primary, 4)?;
    let id_size = match primary_version {
        0 => 2,
        0x0100_0000 => 4,
        _ => return None,
    };
    let primary = number(&mut primary, id_size)?;
    let properties = children(metadata.iter().find(|item| &item.kind == b"iprp")?.data)?;
    let containers = children(properties.iter().find(|item| &item.kind == b"ipco")?.data)?;
    let mut dimensions = None;
    let mut rotation = None;
    for item in properties.iter().filter(|item| &item.kind == b"ipma") {
        for index in primary_properties(item.data, primary)? {
            if index == 0 {
                continue;
            }
            let property = containers.get(usize::from(index - 1))?;
            let mut data = property.data;
            match &property.kind {
                b"ispe" => {
                    if number(&mut data, 4)? != 0 {
                        return None;
                    }
                    let width = i32::try_from(number(&mut data, 4)?).ok()?;
                    let height = i32::try_from(number(&mut data, 4)?).ok()?;
                    if width <= 0 || height <= 0 || dimensions.replace((width, height)).is_some() {
                        return None;
                    }
                }
                b"irot" => {
                    let [angle] = data else { return None };
                    if angle & !3 != 0 || rotation.replace(angle & 1 != 0).is_some() {
                        return None;
                    }
                }
                // Cropped images need the decoder's dimensions, not the uncropped ispe.
                b"clap" => return None,
                _ => {}
            }
        }
    }
    let (width, height) = dimensions?;
    Some(if rotation.unwrap_or(false) {
        (height, width)
    } else {
        (width, height)
    })
}

fn primary_properties(mut data: &[u8], primary: u32) -> Option<Vec<u16>> {
    let version_flags = number(&mut data, 4)?;
    if version_flags & !0x0100_0001 != 0 {
        return None;
    }
    let id_size = if version_flags >> 24 == 0 { 2 } else { 4 };
    let association_size = if version_flags & 1 == 0 { 1 } else { 2 };
    let index_mask = if association_size == 1 { 0x7f } else { 0x7fff };
    let count = number(&mut data, 4)?;
    let mut properties = Vec::new();
    for _ in 0..count {
        let item = number(&mut data, id_size)?;
        let count = usize::try_from(number(&mut data, 1)?).ok()?;
        if item != primary {
            data = data.get(count.checked_mul(association_size)?..)?;
            continue;
        }
        for _ in 0..count {
            let index = number(&mut data, association_size)? & index_mask;
            properties.push(u16::try_from(index).ok()?);
        }
    }
    Some(properties)
}
