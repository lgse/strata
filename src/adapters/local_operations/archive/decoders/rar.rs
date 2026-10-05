// SPDX-License-Identifier: MIT

//! Feeds sandboxed RAR members through the shared extraction session.

use super::super::{
    ArchiveError, archive_failed,
    extraction::{ArchiveOutcome, ExtractionSession, MemberContent, MemberMetadata},
};
use crate::sandbox::archive::{Member, stream_rar};
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
    let outcome = stream_rar(archive_path, password, cancelled, |name, member| {
        let content = match member {
            Member::Directory => MemberContent::Directory,
            Member::File { size, body } => MemberContent::File(body, Some(size)),
        };
        session
            .extract_member(name, content, MemberMetadata::NONE)
            .map_err(|error| error.to_string())
    });
    let result = outcome.map_err(|message| stream_error(message, cancelled));
    session.finish(result, Vec::new)
}

// The child cannot observe the cancellation flag; only the parent classifies it.
fn stream_error(message: String, cancelled: &AtomicBool) -> ArchiveError {
    if cancelled.load(Ordering::Relaxed) {
        ArchiveError::Cancelled
    } else {
        archive_failed(message)
    }
}
