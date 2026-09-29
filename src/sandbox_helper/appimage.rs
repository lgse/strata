// SPDX-License-Identifier: MIT

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
    time::Duration,
};

use tempfile::NamedTempFile;

use super::{bounded_output_with_timeout, render_raw};
use crate::sandbox::MAX_OUTPUT_BYTES;

const LISTING_LIMIT_BYTES: u64 = 1024 * 1024;
const DESKTOP_LIMIT_BYTES: u64 = 64 * 1024;
const LISTING_TIMEOUT: Duration = Duration::from_secs(4);
// Several candidates each stall only on a pathological image; keep every read
// short so the batch stays inside the sandbox wall clock.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(3);
const SQUASHFS_MAGIC: &[u8; 4] = b"hsqs";
const SHT_NOBITS: u32 = 8;
const ICON_EXTENSIONS: [&str; 3] = ["png", "svg", "xpm"];

#[cfg(test)]
mod tests;

pub(super) fn render(input: &Path, size: i32) -> Result<Vec<u8>, String> {
    let mut file = fs::File::open(input).map_err(|error| error.to_string())?;
    let offset = embedded_image_offset(&mut file)
        .ok_or_else(|| "No SquashFS image follows the ELF runtime".to_owned())?;
    let listing = unsquashfs(
        input,
        offset,
        "-ll",
        None,
        LISTING_LIMIT_BYTES,
        LISTING_TIMEOUT,
    )?;
    let listing = String::from_utf8_lossy(&listing);
    let entries = root_entries(&listing);
    let mut candidates = dir_icon_candidates(&entries);
    if let Some(desktop) = entries
        .iter()
        .find(|entry| entry.is_file && entry.name.ends_with(".desktop"))
        && let Ok(contents) = unsquashfs(
            input,
            offset,
            "-cat",
            Some(desktop.name.as_str()),
            DESKTOP_LIMIT_BYTES,
            EXTRACT_TIMEOUT,
        )
        && let Some(icon) = desktop_icon(&contents)
    {
        candidates.extend(icon_name_candidates(&entries, &icon));
    }
    for candidate in candidates {
        if let Ok(icon) = unsquashfs(
            input,
            offset,
            "-cat",
            Some(candidate.as_str()),
            MAX_OUTPUT_BYTES,
            EXTRACT_TIMEOUT,
        ) && let Ok(png) = render_icon(&icon, size)
        {
            return Ok(png);
        }
    }
    Err("No usable icon inside the AppImage".to_owned())
}

fn unsquashfs(
    input: &Path,
    offset: u64,
    mode: &str,
    member: Option<&str>,
    max_bytes: u64,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut command = Command::new("unsquashfs");
    command
        .arg("-o")
        .arg(offset.to_string())
        .arg(mode)
        .arg(input);
    if let Some(member) = member {
        command.arg(member);
    }
    let output = bounded_output_with_timeout(&mut command, max_bytes, timeout)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "SquashFS read timed out".to_owned())?;
    if !output.status.success() {
        return Err("unsquashfs rejected the embedded image".to_owned());
    }
    Ok(output.stdout)
}

fn render_icon(bytes: &[u8], size: i32) -> Result<Vec<u8>, String> {
    let temp = NamedTempFile::new().map_err(|error| error.to_string())?;
    fs::write(temp.path(), bytes).map_err(|error| error.to_string())?;
    render_raw(temp.path(), size)
}

struct RootEntry {
    name: String,
    is_file: bool,
    symlink_target: Option<String>,
}

fn root_entries(listing: &str) -> Vec<RootEntry> {
    const ROOT: &str = "squashfs-root/";
    listing
        .lines()
        .filter_map(|line| {
            let start = line.find(ROOT)? + ROOT.len();
            let kind = line.as_bytes().first().copied()?;
            let (path, target) = line[start..]
                .split_once(" -> ")
                .map_or((&line[start..], None), |(path, target)| {
                    (path, Some(target))
                });
            if path.is_empty() || path.contains('/') {
                return None;
            }
            Some(RootEntry {
                name: path.to_owned(),
                is_file: kind == b'-',
                symlink_target: target.map(str::to_owned),
            })
        })
        .collect()
}

fn dir_icon_candidates(entries: &[RootEntry]) -> Vec<String> {
    let Some(dir_icon) = entries.iter().find(|entry| entry.name == ".DirIcon") else {
        return Vec::new();
    };
    if dir_icon.is_file {
        return vec![dir_icon.name.clone()];
    }
    let Some(target) = &dir_icon.symlink_target else {
        return Vec::new();
    };
    if !target.starts_with('/') {
        return vec![target.trim_start_matches("./").to_owned()];
    }
    // AppImage builders leave absolute build-host targets; the basename still
    // names the icon beside the launcher.
    let Some(base) = target.rsplit('/').next() else {
        return Vec::new();
    };
    entries
        .iter()
        .find(|entry| entry.is_file && entry.name.eq_ignore_ascii_case(base))
        .map(|entry| vec![entry.name.clone()])
        .unwrap_or_default()
}

fn icon_name_candidates(entries: &[RootEntry], icon: &str) -> Vec<String> {
    if icon.contains('/') {
        return vec![icon.trim_start_matches('/').to_owned()];
    }
    entries
        .iter()
        .filter(|entry| {
            entry.is_file
                && entry
                    .name
                    .rsplit_once('.')
                    .is_some_and(|(stem, extension)| {
                        (stem.eq_ignore_ascii_case(icon) || entry.name.eq_ignore_ascii_case(icon))
                            && ICON_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
                    })
        })
        .map(|entry| entry.name.clone())
        .collect()
}

fn desktop_icon(contents: &[u8]) -> Option<String> {
    std::str::from_utf8(contents)
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("Icon=")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
}

fn embedded_image_offset(reader: &mut (impl Read + Seek)) -> Option<u64> {
    let mut header = [0u8; 64];
    reader.read_exact(&mut header).ok()?;
    if header[..4] != *b"\x7fELF" {
        return None;
    }
    let class64 = match header[4] {
        1 => false,
        2 => true,
        _ => return None,
    };
    let big_endian = match header[5] {
        1 => false,
        2 => true,
        _ => return None,
    };
    let minimum = if class64 { 64 } else { 40 };
    let (shoff, shentsize, shnum) = if class64 {
        (
            uint(&header, 0x28, 8, big_endian),
            uint(&header, 0x3a, 2, big_endian),
            uint(&header, 0x3c, 2, big_endian),
        )
    } else {
        (
            uint(&header, 0x20, 4, big_endian),
            uint(&header, 0x2e, 2, big_endian),
            uint(&header, 0x30, 2, big_endian),
        )
    };
    if shoff == 0 || (shentsize as usize) < minimum {
        return None;
    }
    let mut section = vec![0u8; shentsize as usize];
    reader.seek(SeekFrom::Start(shoff)).ok()?;
    let mut count = shnum;
    if count == 0 {
        // Extended numbering keeps the real count in section 0's sh_size.
        reader.read_exact(&mut section).ok()?;
        count = section_header(&section, class64, big_endian).size;
        reader.seek(SeekFrom::Start(shoff)).ok()?;
    }
    if count == 0 || count > 4096 {
        return None;
    }
    let mut end = shoff.saturating_add(shentsize.saturating_mul(count));
    for _ in 0..count {
        reader.read_exact(&mut section).ok()?;
        let section = section_header(&section, class64, big_endian);
        if section.kind != SHT_NOBITS {
            end = end.max(section.offset.saturating_add(section.size));
        }
    }
    reader.seek(SeekFrom::Start(end)).ok()?;
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic).ok()?;
    (magic == *SQUASHFS_MAGIC).then_some(end)
}

struct SectionHeader {
    kind: u32,
    offset: u64,
    size: u64,
}

fn section_header(bytes: &[u8], class64: bool, big_endian: bool) -> SectionHeader {
    if class64 {
        SectionHeader {
            kind: uint(bytes, 4, 4, big_endian) as u32,
            offset: uint(bytes, 24, 8, big_endian),
            size: uint(bytes, 32, 8, big_endian),
        }
    } else {
        SectionHeader {
            kind: uint(bytes, 4, 4, big_endian) as u32,
            offset: uint(bytes, 16, 4, big_endian),
            size: uint(bytes, 20, 4, big_endian),
        }
    }
}

fn uint(bytes: &[u8], offset: usize, width: usize, big_endian: bool) -> u64 {
    let Some(field) = bytes.get(offset..offset + width) else {
        return 0;
    };
    let order: Box<dyn Iterator<Item = &u8>> = if big_endian {
        Box::new(field.iter())
    } else {
        Box::new(field.iter().rev())
    };
    order.fold(0, |value, byte| value << 8 | u64::from(*byte))
}
