// SPDX-License-Identifier: MIT

//! Drives the sandboxed RAR-extraction helper (see [`crate::sandbox::archive`])
//! and feeds its member stream into the same [`ExtractionSession`] every other
//! archive format uses. No UnRAR call happens in this process; the FFI code
//! that does lives only in [`crate::sandbox_helper::archive_rar`], confined to
//! the bubblewrapped child.

use super::super::{
    ArchiveError, archive_failed,
    extraction::{ArchiveOutcome, ExtractedRoots, ExtractionSession, MemberContent},
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
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<ExtractedRoots>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, progress, cancelled)?;
    let outcome = stream_rar(archive_path, password, cancelled, |name, member| {
        let content = match member {
            Member::Directory => MemberContent::Directory,
            Member::File { size, body } => MemberContent::File(body, Some(size)),
        };
        session
            .extract_member(name, content)
            .map_err(|error| error.to_string())
    });
    let result = outcome.map_err(|message| stream_error(message, cancelled));
    session.finish(result, Vec::new)
}

/// The sandboxed stream reports every failure as a plain message, including
/// one caused by `cancelled` itself (the child has no way to know it was
/// asked to stop); `cancelled` is the single source of truth for whether an
/// error means the operation was cancelled, matching how every other decoder
/// in this module already treats that flag.
fn stream_error(message: String, cancelled: &AtomicBool) -> ArchiveError {
    if cancelled.load(Ordering::Relaxed) {
        ArchiveError::Cancelled
    } else {
        archive_failed(message)
    }
}
