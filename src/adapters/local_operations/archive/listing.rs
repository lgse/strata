// SPDX-License-Identifier: MIT

//! Lists metadata without extracting members. TAR.GZ must decompress past file data;
//! readable 7z headers remain listable even when their file contents are encrypted.

use crate::services::{ArchiveFileEntry, ArchiveFormat};
use std::{
    cell::Cell,
    fs::File,
    io::{Read, Seek},
    path::Path,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
mod tests;

pub(crate) const INVALID_ARCHIVE: &str = "This file is not a valid archive or is damaged.";

pub(crate) const ARCHIVE_UNSUPPORTED_MESSAGE: &str =
    "This archive format is not supported for preview.";

pub(crate) const ARCHIVE_PREVIEW_FAILED_MESSAGE: &str = "Preview couldn't be completed. Try again.";

pub(crate) const MAX_ARCHIVE_ENTRIES: usize = 20_000;
pub(crate) const MAX_TAR_GZ_COMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const MAX_TAR_GZ_DECOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const ARCHIVE_TOO_LARGE_MESSAGE: &str = "Archive too large to preview.";

pub(crate) const MAX_ARCHIVE_PASSWORD_BYTES: usize = 4096;

const WIRE_OPEN: &str = "open";
const WIRE_NEEDS_PASSWORD: &str = "needs-password";
const WIRE_WRONG_PASSWORD: &str = "wrong-password";
const WIRE_UNSUPPORTED: &str = "unsupported";
const WIRE_TOO_LARGE: &str = "too-large";
const WIRE_ERROR: &str = "error";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArchiveListingStatus {
    Open,
    NeedsPassword,
    WrongPassword,
    Unsupported,
}

pub(crate) struct ArchiveListing {
    pub status: ArchiveListingStatus,
    pub entries: Vec<ArchiveFileEntry>,
}

impl ArchiveListing {
    fn open(entries: Vec<ArchiveFileEntry>) -> Self {
        Self {
            status: ArchiveListingStatus::Open,
            entries,
        }
    }
}

/// Parses untrusted input with ambient authority; production callers must run inside the sandbox.
pub(crate) fn list_archive_entries_direct(
    archive_path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<ArchiveListing, String> {
    match format {
        ArchiveFormat::Zip => list_zip(archive_path, password, cancelled),
        ArchiveFormat::SevenZ => list_7z(archive_path, password, cancelled),
        ArchiveFormat::Tar => list_tar(archive_path, false, cancelled),
        ArchiveFormat::TarGz => list_tar(archive_path, true, cancelled),
        ArchiveFormat::Rar => Err(ARCHIVE_UNSUPPORTED_MESSAGE.to_owned()),
    }
}

fn ensure_entry_budget(accepted: usize) -> Result<(), String> {
    if accepted >= MAX_ARCHIVE_ENTRIES {
        return Err(ARCHIVE_TOO_LARGE_MESSAGE.to_owned());
    }
    Ok(())
}

// Entry counts alone do not bound the memory used by synthesized parent directories.
const MAX_ARCHIVE_NAME_BYTES: usize = 16 * 1024;
const MAX_ARCHIVE_ENTRY_SEGMENTS: usize = 4096;
const MAX_ARCHIVE_TOTAL_NAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_ARCHIVE_TOTAL_SEGMENTS: usize = 512 * 1024;

fn entry_name_segments(name: &str) -> usize {
    crate::services::split_archive_name(name).len()
}

// Enforce on both sides of the sandbox before building a tree in the parent.
#[derive(Default)]
struct MemberNameBudget {
    name_bytes: usize,
    segments: usize,
}

impl MemberNameBudget {
    fn check(&mut self, name: &str) -> Result<(), String> {
        if name.len() > MAX_ARCHIVE_NAME_BYTES
            || self.name_bytes.saturating_add(name.len()) > MAX_ARCHIVE_TOTAL_NAME_BYTES
        {
            return Err(ARCHIVE_TOO_LARGE_MESSAGE.to_owned());
        }
        let segments = entry_name_segments(name);
        if segments > MAX_ARCHIVE_ENTRY_SEGMENTS
            || self.segments.saturating_add(segments) > MAX_ARCHIVE_TOTAL_SEGMENTS
        {
            return Err(ARCHIVE_TOO_LARGE_MESSAGE.to_owned());
        }
        self.name_bytes += name.len();
        self.segments += segments;
        Ok(())
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Preview cancelled".to_owned())
    } else {
        Ok(())
    }
}

fn zip_error(error: zip::result::ZipError) -> String {
    use zip::result::ZipError;
    match error {
        ZipError::UnsupportedArchive(_) | ZipError::CompressionMethodNotSupported(_) => {
            ARCHIVE_UNSUPPORTED_MESSAGE.to_owned()
        }
        _ => INVALID_ARCHIVE.to_owned(),
    }
}

fn sevenz_list_error(error: sevenz_rust2::Error) -> String {
    use sevenz_rust2::Error;
    match error {
        Error::Unsupported(_) => ARCHIVE_UNSUPPORTED_MESSAGE.to_owned(),
        _ => INVALID_ARCHIVE.to_owned(),
    }
}

fn list_zip(
    archive_path: &Path,
    password: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<ArchiveListing, String> {
    let file = File::open(archive_path).map_err(|error| error.to_string())?;
    let mut central_directory = file.try_clone().map_err(|_| invalid_archive())?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    // zip indexes by name and silently overwrites duplicates. Refuse an incomplete preview.
    let member_count = zip_member_count(
        &mut central_directory,
        archive.central_directory_start(),
        cancelled,
    )?;
    if member_count != archive.len() {
        return Err(ARCHIVE_UNSUPPORTED_MESSAGE.to_owned());
    }
    let mut entries = Vec::with_capacity(archive.len().min(MAX_ARCHIVE_ENTRIES));
    let mut first_encrypted = None;
    let mut first_encrypted_file = None;
    let mut budget = MemberNameBudget::default();
    for index in 0..archive.len() {
        check_cancelled(cancelled)?;
        // Central-directory metadata is readable before password validation.
        let member = archive.by_index_raw(index).map_err(zip_error)?;
        if member.encrypted() {
            first_encrypted.get_or_insert(index);
            if !member.is_dir() {
                first_encrypted_file.get_or_insert(index);
            }
        }
        ensure_entry_budget(entries.len())?;
        budget.check(member.name())?;
        entries.push(ArchiveFileEntry {
            name: member.name().to_owned(),
            directory: member.is_dir(),
            size: member.size(),
        });
    }
    let Some(index) = first_encrypted_file.or(first_encrypted) else {
        return Ok(ArchiveListing::open(entries));
    };
    let Some(password) = password else {
        return Ok(ArchiveListing {
            status: ArchiveListingStatus::NeedsPassword,
            entries,
        });
    };
    // Opening a protected member validates the decryption header immediately;
    // no member data is produced.
    match archive.by_index_with_options(
        index,
        zip::read::ZipReadOptions::new().password(Some(password.as_bytes())),
    ) {
        Ok(_) => Ok(ArchiveListing::open(entries)),
        Err(zip::result::ZipError::InvalidPassword) => Ok(ArchiveListing {
            status: ArchiveListingStatus::WrongPassword,
            entries,
        }),
        Err(zip::result::ZipError::UnsupportedArchive(_)) => Ok(ArchiveListing {
            status: ArchiveListingStatus::Unsupported,
            entries,
        }),
        Err(error) => Err(zip_error(error)),
    }
}

fn zip_member_count(
    reader: &mut (impl Read + Seek),
    offset: u64,
    cancelled: &AtomicBool,
) -> Result<usize, String> {
    use std::io::SeekFrom;

    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| invalid_archive())?;
    let mut count = 0;
    loop {
        check_cancelled(cancelled)?;
        let mut signature = [0; 4];
        reader
            .read_exact(&mut signature)
            .map_err(|_| invalid_archive())?;
        match signature {
            [0x50, 0x4b, 0x01, 0x02] => {}
            [0x50, 0x4b, 0x05, 0x06] | [0x50, 0x4b, 0x06, 0x06] | [0x50, 0x4b, 0x05, 0x05] => {
                return Ok(count);
            }
            _ => return Err(invalid_archive()),
        }
        ensure_entry_budget(count)?;
        let mut header = [0; 42];
        reader
            .read_exact(&mut header)
            .map_err(|_| invalid_archive())?;
        // Central-directory records end in variable-length name, extra, and comment fields.
        let trailing_bytes: u32 = [24, 26, 28]
            .into_iter()
            .map(|index| u32::from(u16::from_le_bytes([header[index], header[index + 1]])))
            .sum();
        reader
            .seek(SeekFrom::Current(i64::from(trailing_bytes)))
            .map_err(|_| invalid_archive())?;
        count += 1;
    }
}

fn list_7z(
    archive_path: &Path,
    password: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<ArchiveListing, String> {
    let file = File::open(archive_path).map_err(|error| error.to_string())?;
    let password = password
        .map(sevenz_rust2::Password::new)
        .unwrap_or_else(sevenz_rust2::Password::empty);
    use sevenz_rust2::Error;
    let archive = match sevenz_rust2::ArchiveReader::new(file, password) {
        Ok(archive) => archive,
        Err(Error::PasswordRequired) => {
            return Ok(ArchiveListing {
                status: ArchiveListingStatus::NeedsPassword,
                entries: Vec::new(),
            });
        }
        Err(Error::MaybeBadPassword(_)) => {
            return Ok(ArchiveListing {
                status: ArchiveListingStatus::WrongPassword,
                entries: Vec::new(),
            });
        }
        Err(Error::Unsupported(_)) => {
            return Ok(ArchiveListing {
                status: ArchiveListingStatus::Unsupported,
                entries: Vec::new(),
            });
        }
        Err(error) => return Err(sevenz_list_error(error)),
    };
    let mut entries = Vec::with_capacity(archive.archive().files.len().min(MAX_ARCHIVE_ENTRIES));
    let mut budget = MemberNameBudget::default();
    for member in &archive.archive().files {
        check_cancelled(cancelled)?;
        ensure_entry_budget(entries.len())?;
        budget.check(&member.name)?;
        entries.push(ArchiveFileEntry {
            name: member.name.clone(),
            directory: member.is_directory,
            size: member.size,
        });
    }
    Ok(ArchiveListing::open(entries))
}

fn list_tar(
    archive_path: &Path,
    gzip: bool,
    cancelled: &AtomicBool,
) -> Result<ArchiveListing, String> {
    let file = File::open(archive_path).map_err(|error| error.to_string())?;
    if gzip {
        if file.metadata().map(|metadata| metadata.len()).unwrap_or(0) > MAX_TAR_GZ_COMPRESSED_BYTES
        {
            return Err(ARCHIVE_TOO_LARGE_MESSAGE.to_owned());
        }
        let decoder = flate2::read::GzDecoder::new(file);
        let reader = BudgetReader {
            inner: decoder,
            remaining: MAX_TAR_GZ_DECOMPRESSED_BYTES,
            cancelled,
        };
        let mut archive = tar::Archive::new(reader);
        let entries = archive.entries().map_err(tar_list_error)?;
        collect_tar_members(entries, cancelled)
    } else {
        let file_len = file
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(u64::MAX);
        let furthest_seek = Rc::new(Cell::new(0u64));
        let tracker = SeekTargetTracker {
            inner: file,
            furthest: furthest_seek.clone(),
        };
        let listing = collect_tar_seekable(tracker, cancelled)?;
        if furthest_seek.get() > file_len {
            return Err(INVALID_ARCHIVE.to_owned());
        }
        Ok(listing)
    }
}

struct BudgetReader<'a, R> {
    inner: R,
    remaining: u64,
    cancelled: &'a AtomicBool,
}

impl<R: Read> Read for BudgetReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("Preview cancelled"));
        }
        if self.remaining == 0 {
            let read = self.inner.read(buf)?;
            if read == 0 {
                return Ok(0);
            }
            return Err(std::io::Error::other(ARCHIVE_TOO_LARGE_MESSAGE));
        }
        let allowed = (buf.len() as u64).min(self.remaining) as usize;
        let read = self.inner.read(&mut buf[..allowed])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

// Seeking past EOF succeeds, so header iteration alone cannot detect truncated member data.
struct SeekTargetTracker<R> {
    inner: R,
    furthest: Rc<Cell<u64>>,
}

impl<R: Read> Read for SeekTargetTracker<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for SeekTargetTracker<R> {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        let position = self.inner.seek(position)?;
        self.furthest.set(self.furthest.get().max(position));
        Ok(position)
    }
}

fn tar_list_error(error: std::io::Error) -> String {
    let message = error.to_string();
    if message == ARCHIVE_TOO_LARGE_MESSAGE || message == "Preview cancelled" {
        message
    } else {
        INVALID_ARCHIVE.to_owned()
    }
}

// Unlike entries(), entries_with_seek() skips file data without reading it.
fn collect_tar_seekable<R: Read + Seek>(
    reader: R,
    cancelled: &AtomicBool,
) -> Result<ArchiveListing, String> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries_with_seek().map_err(tar_list_error)?;
    collect_tar_members(entries, cancelled)
}

fn collect_tar_members<'a, R: Read + 'a>(
    entries: tar::Entries<'a, R>,
    cancelled: &AtomicBool,
) -> Result<ArchiveListing, String> {
    let mut listed = Vec::new();
    let mut budget = MemberNameBudget::default();
    for member in entries {
        check_cancelled(cancelled)?;
        let member = member.map_err(tar_list_error)?;
        if matches!(
            member.header().entry_type(),
            tar::EntryType::XGlobalHeader
                | tar::EntryType::XHeader
                | tar::EntryType::GNULongName
                | tar::EntryType::GNULongLink
        ) {
            continue;
        }
        let name = member.path().map_err(tar_list_error)?;
        let directory = member.header().entry_type().is_dir();
        if directory && name == Path::new(".") {
            continue;
        }
        let name = name.to_string_lossy().into_owned();
        budget.check(&name)?;
        ensure_entry_budget(listed.len())?;
        listed.push(ArchiveFileEntry {
            name,
            directory,
            size: member.size(),
        });
    }
    Ok(ArchiveListing::open(listed))
}

pub(crate) fn encode_archive_result(result: &Result<ArchiveListing, String>) -> Vec<u8> {
    let (status, entries, message): (&str, &[ArchiveFileEntry], Option<&str>) = match result {
        Ok(listing) => (
            match listing.status {
                ArchiveListingStatus::Open => WIRE_OPEN,
                ArchiveListingStatus::NeedsPassword => WIRE_NEEDS_PASSWORD,
                ArchiveListingStatus::WrongPassword => WIRE_WRONG_PASSWORD,
                ArchiveListingStatus::Unsupported => WIRE_UNSUPPORTED,
            },
            &listing.entries,
            None,
        ),
        Err(message) if message == ARCHIVE_TOO_LARGE_MESSAGE => (WIRE_TOO_LARGE, &[], None),
        Err(message) => (WIRE_ERROR, &[], Some(message.as_str())),
    };
    let entries: Vec<serde_json::Value> = entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "name": entry.name,
                "directory": entry.directory,
                "size": entry.size,
            })
        })
        .collect();
    serde_json::json!({
        "status": status,
        "entries": entries,
        "message": message,
    })
    .to_string()
    .into_bytes()
}

fn invalid_archive() -> String {
    INVALID_ARCHIVE.to_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WireStatus {
    Open,
    NeedsPassword,
    WrongPassword,
    Unsupported,
    TooLarge,
    Error,
}

impl WireStatus {
    fn parse(status: &str) -> Option<Self> {
        match status {
            WIRE_OPEN => Some(Self::Open),
            WIRE_NEEDS_PASSWORD => Some(Self::NeedsPassword),
            WIRE_WRONG_PASSWORD => Some(Self::WrongPassword),
            WIRE_UNSUPPORTED => Some(Self::Unsupported),
            WIRE_TOO_LARGE => Some(Self::TooLarge),
            WIRE_ERROR => Some(Self::Error),
            _ => None,
        }
    }
}

// Error statuses are valid payloads; rejecting them here would hide the intended error.
struct ArchivePayload {
    pub status: WireStatus,
    pub entries: Vec<ArchiveFileEntry>,
    pub message: Option<String>,
}

fn decode_archive_payload(data: &[u8]) -> Result<ArchivePayload, String> {
    let value: serde_json::Value = serde_json::from_slice(data).map_err(|_| invalid_archive())?;
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .and_then(WireStatus::parse)
        .ok_or_else(invalid_archive)?;
    let entries = value
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(invalid_archive)?;
    if entries.len() > MAX_ARCHIVE_ENTRIES {
        return Err(invalid_archive());
    }
    let mut listed = Vec::with_capacity(entries.len().min(MAX_ARCHIVE_ENTRIES));
    for entry in entries {
        let name = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(invalid_archive)?;
        let directory = entry
            .get("directory")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(invalid_archive)?;
        let size = entry
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(invalid_archive)?;
        listed.push(ArchiveFileEntry {
            name: name.to_owned(),
            directory,
            size,
        });
    }
    let message = value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if status == WireStatus::Error && message.as_deref().is_none_or(str::is_empty) {
        return Err(invalid_archive());
    }
    Ok(ArchivePayload {
        status,
        entries: listed,
        message,
    })
}

pub(crate) fn archive_payload_valid(data: &[u8]) -> bool {
    decode_archive_payload(data).is_ok()
}

pub(crate) fn decode_archive_listing(data: &[u8]) -> Result<ArchiveListing, String> {
    let payload = decode_archive_payload(data)?;
    let mut budget = MemberNameBudget::default();
    for entry in &payload.entries {
        budget.check(&entry.name)?;
    }
    match payload.status {
        WireStatus::Open => Ok(ArchiveListing::open(payload.entries)),
        WireStatus::NeedsPassword => Ok(ArchiveListing {
            status: ArchiveListingStatus::NeedsPassword,
            entries: payload.entries,
        }),
        WireStatus::WrongPassword => Ok(ArchiveListing {
            status: ArchiveListingStatus::WrongPassword,
            entries: payload.entries,
        }),
        WireStatus::Unsupported => Ok(ArchiveListing {
            status: ArchiveListingStatus::Unsupported,
            entries: payload.entries,
        }),
        WireStatus::TooLarge => Err(ARCHIVE_TOO_LARGE_MESSAGE.to_owned()),
        WireStatus::Error => Err(payload
            .message
            .filter(|message| !message.is_empty())
            .ok_or_else(invalid_archive)?),
    }
}
