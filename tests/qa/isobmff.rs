// SPDX-License-Identifier: MIT

use std::{fs::File, io::Read, path::Path};

/// ImageMagick without an encoder for the requested format exits 0 and writes the
/// input's format under the requested name, so these outputs must carry the brand.
/// `.heif` names no codec, so it accepts `mif1`, the generic HEIF image brand.
const ISOBMFF_BRANDS: &[(&str, &[&[u8; 4]])] = &[
    ("avif", &[b"avif", b"avis"]),
    ("heic", &[b"heic", b"heix"]),
    ("heif", &[b"mif1", b"heic", b"heix"]),
];

const MAX_FTYP_PAYLOAD_BYTES: u64 = 4096;

fn ftyp_brands(path: &Path) -> Result<Vec<[u8; 4]>, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let mut header = [0; 8];
    file.read_exact(&mut header)
        .map_err(|error| format!("no box header: {error}"))?;
    if &header[4..] != b"ftyp" {
        return Err("no leading ftyp box".to_owned());
    }
    let (size, header_size) = match u32::from_be_bytes([header[0], header[1], header[2], header[3]])
    {
        0 => (length, 8),
        1 => {
            let mut large = [0; 8];
            file.read_exact(&mut large)
                .map_err(|error| format!("no ftyp largesize: {error}"))?;
            (u64::from_be_bytes(large), 16)
        }
        size => (u64::from(size), 8),
    };
    let payload = size
        .checked_sub(header_size)
        .filter(|payload| {
            size <= length && (8..=MAX_FTYP_PAYLOAD_BYTES).contains(payload) && payload % 4 == 0
        })
        .ok_or_else(|| format!("malformed ftyp box of {size} bytes"))?;
    let mut data = vec![0; usize::try_from(payload).map_err(|error| error.to_string())?];
    file.read_exact(&mut data)
        .map_err(|error| format!("short ftyp box: {error}"))?;
    let minor_version = 1;
    Ok(data
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != minor_version)
        .map(|(_, brand)| *brand)
        .collect())
}

pub(crate) fn check_brand(path: &Path) -> Result<(), String> {
    let extension = path.extension().and_then(|extension| extension.to_str());
    let Some((name, expected)) = ISOBMFF_BRANDS
        .iter()
        .find(|(name, _)| extension.is_some_and(|extension| extension.eq_ignore_ascii_case(name)))
    else {
        return Ok(());
    };
    let unbranded = |reason: &str| {
        format!(
            "ImageMagick wrote {} without an ISO BMFF {name} brand ({reason}); the pinned \
             image lacks its {name} encoder",
            path.display()
        )
    };
    let brands = ftyp_brands(path).map_err(|reason| unbranded(&reason))?;
    let branded = brands.iter().any(|brand| expected.contains(&brand));
    branded
        .then_some(())
        .ok_or_else(|| unbranded("no matching major or compatible brand"))
}
