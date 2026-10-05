// SPDX-License-Identifier: MIT

//! Feeds sandboxed RAR members through the shared extraction session.

use super::super::{
    ArchiveError, archive_failed,
    extraction::{ArchiveOutcome, ExtractionSession, MemberContent, MemberMetadata},
};
use super::{dos_local_time, filetime, member_mode};
use crate::{
    rar_extraction::{WireMetadata, WireTime},
    sandbox::archive::{Member, stream_rar},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[cfg(test)]
mod tests;

pub(in crate::adapters::local_operations::archive) fn extract_rar(
    archive_path: &Path,
    dest_dir: &Path,
    archive_name: &str,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, archive_name, progress, cancelled)?;
    let outcome = stream_rar(
        archive_path,
        password,
        cancelled,
        |name, member, metadata| {
            let directory = matches!(member, Member::Directory);
            let content = match member {
                Member::Directory => MemberContent::Directory,
                Member::File { size, body } => MemberContent::File(body, Some(size)),
            };
            session
                .extract_member(name, content, member_metadata(metadata, directory))
                .map_err(|error| error.to_string())
        },
    );
    let result = outcome.map_err(|message| stream_error(message, cancelled));
    session.finish(result, Vec::new)
}

/// RAR links still arrive as files, so their link mode must not be applied.
fn member_metadata(metadata: WireMetadata, directory: bool) -> MemberMetadata {
    MemberMetadata {
        mode: member_mode(metadata.mode, directory),
        modified: metadata.modified.and_then(|time| match time {
            WireTime::FileTime(value) => filetime(value),
            // RAR stores DOS dates in the same layout as ZIP.
            WireTime::DosLocal(value) => {
                zip::DateTime::try_from_msdos((value >> 16) as u16, value as u16)
                    .ok()
                    .and_then(dos_local_time)
            }
        }),
    }
}

// The child cannot observe the cancellation flag; only the parent classifies it.
fn stream_error(message: String, cancelled: &AtomicBool) -> ArchiveError {
    if cancelled.load(Ordering::Relaxed) {
        ArchiveError::Cancelled
    } else {
        archive_failed(message)
    }
}
