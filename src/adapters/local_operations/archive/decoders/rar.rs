// SPDX-License-Identifier: MIT

//! Feeds sandboxed RAR members through the shared extraction session.

use super::super::{
    ArchiveError, archive_failed,
    extraction::{ArchiveOutcome, ExtractionSession, MemberContent, MemberMetadata},
};
use super::{dos_local_time, filetime, member_mode};
use crate::{
    rar_extraction::{Failure, FailureKind, WireMetadata, WireTime},
    sandbox::archive::{Member, stream_rar},
};
use std::{
    io::Read,
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
    // The stream's callback errors carry only text; retain the structured kind separately.
    let mut member_error = None;
    let outcome = stream_rar(
        archive_path,
        password,
        cancelled,
        |name, member, metadata| {
            let directory = matches!(member, Member::Directory);
            let mut body;
            let content = match member {
                Member::Directory => MemberContent::Directory,
                Member::File { size, body: reader } => {
                    body = MemberBody(reader);
                    MemberContent::File(&mut body, Some(size))
                }
            };
            session
                .extract_member(name, content, member_metadata(metadata, directory))
                .map_err(|error| {
                    let message = error.to_string();
                    member_error = Some(error);
                    message
                })
        },
    );
    let result = outcome.map_err(|failure| stream_error(failure, member_error, cancelled));
    session.finish(result, Vec::new)
}

struct MemberBody<'a>(&'a mut dyn Read);

impl Read for MemberBody<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer).map_err(|error| {
            match error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<Failure>())
            {
                Some(failure) => std::io::Error::new(error.kind(), archive_error(failure.clone())),
                None => error,
            }
        })
    }
}

fn archive_error(failure: Failure) -> ArchiveError {
    match failure.kind {
        FailureKind::Other => archive_failed(failure.message),
        FailureKind::PasswordRequired => ArchiveError::PasswordRequired(failure.message),
        FailureKind::IncorrectPassword => ArchiveError::IncorrectPassword(failure.message),
    }
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
fn stream_error(
    failure: Failure,
    member_error: Option<ArchiveError>,
    cancelled: &AtomicBool,
) -> ArchiveError {
    if cancelled.load(Ordering::Relaxed) {
        ArchiveError::Cancelled
    } else {
        member_error.unwrap_or_else(|| archive_error(failure))
    }
}
