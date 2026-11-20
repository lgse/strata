// SPDX-License-Identifier: MIT

//! The archive listing a sandboxed helper writes and the launcher validates and decodes.

use crate::services::ArchiveFileEntry;

#[cfg(test)]
mod tests;

pub const INVALID_ARCHIVE: &str = "This file is not a valid archive or is damaged.";

pub const ARCHIVE_UNSUPPORTED_MESSAGE: &str = "This archive format is not supported for preview.";

pub const ARCHIVE_PREVIEW_FAILED_MESSAGE: &str = "Preview couldn't be completed. Try again.";

pub const MAX_ARCHIVE_ENTRIES: usize = 20_000;

pub const ARCHIVE_TOO_LARGE_MESSAGE: &str = "Archive too large to preview.";

pub const MAX_ARCHIVE_PASSWORD_BYTES: usize = 4096;

pub const PASSWORD_REQUIRED: &str = "A password is required to extract this archive.";
/// 7z/RAR and failed decryption checks cannot always distinguish damage from a bad password.
pub const MAYBE_BAD_PASSWORD: &str = "The password may be incorrect.";

const WIRE_OPEN: &str = "open";
const WIRE_NEEDS_PASSWORD: &str = "needs-password";
const WIRE_WRONG_PASSWORD: &str = "wrong-password";
const WIRE_UNSUPPORTED: &str = "unsupported";
const WIRE_TOO_LARGE: &str = "too-large";
const WIRE_ERROR: &str = "error";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveListingStatus {
    Open,
    NeedsPassword,
    WrongPassword,
    Unsupported,
}

pub struct ArchiveListing {
    pub status: ArchiveListingStatus,
    pub entries: Vec<ArchiveFileEntry>,
}

impl ArchiveListing {
    pub fn open(entries: Vec<ArchiveFileEntry>) -> Self {
        Self {
            status: ArchiveListingStatus::Open,
            entries,
        }
    }
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
pub struct MemberNameBudget {
    name_bytes: usize,
    segments: usize,
}

impl MemberNameBudget {
    pub fn check(&mut self, name: &str) -> Result<(), String> {
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

pub fn encode_archive_result(result: &Result<ArchiveListing, String>) -> Vec<u8> {
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

pub fn invalid_archive() -> String {
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

pub fn archive_payload_valid(data: &[u8]) -> bool {
    decode_archive_payload(data).is_ok()
}

pub fn decode_archive_listing(data: &[u8]) -> Result<ArchiveListing, String> {
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
