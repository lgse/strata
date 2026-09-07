// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-operation extraction policy, independent of archive decoding libraries.
//!
//! Decoders lend member streams to the session and supply already-known remaining
//! locations only on cancellation. The session never scans ahead in an archive.

use std::{
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::model::Location;

use super::{
    ArchiveError, check_archive_cancelled, copy_with_big_buf,
    destination::{ExtractNameResolver, ExtractionDestination, validated_archive_path},
};

#[cfg(test)]
mod tests;

pub(super) enum MemberContent<'a> {
    Directory,
    File(&'a mut dyn Read),
}

/// Result of an extract that may stop after writing some members.
pub(super) enum ArchiveOutcome<T> {
    Completed(T),
    Cancelled {
        completed: Vec<Location>,
        failed: Vec<Location>,
        not_attempted: Vec<Location>,
    },
}

enum InterruptedMember {
    NotAttempted(Location),
    Failed(Location),
}

pub(super) struct ExtractionSession<'a> {
    destination: &'a Path,
    directory: ExtractionDestination,
    resolver: ExtractNameResolver,
    progress: &'a AtomicUsize,
    cancelled: &'a AtomicBool,
    first_name: Option<String>,
    completed: Vec<Location>,
    interrupted: Option<InterruptedMember>,
}

impl<'a> ExtractionSession<'a> {
    pub(super) fn open(
        destination: &'a Path,
        progress: &'a AtomicUsize,
        cancelled: &'a AtomicBool,
    ) -> Result<Self, ArchiveError> {
        Ok(Self {
            destination,
            directory: ExtractionDestination::open(destination)?,
            resolver: ExtractNameResolver::new(),
            progress,
            cancelled,
            first_name: None,
            completed: Vec::new(),
            interrupted: None,
        })
    }

    /// Allows a decoder to stop before opening/decrypting another member.
    pub(super) fn check_cancelled(&self) -> Result<(), ArchiveError> {
        check_archive_cancelled(self.cancelled)
    }

    /// Processes one member. On error the decoder must stop and call `finish`.
    pub(super) fn extract_member(
        &mut self,
        name: &str,
        content: MemberContent<'_>,
    ) -> Result<(), ArchiveError> {
        let path = validated_archive_path(name)?;
        if let Err(error) = self.check_cancelled() {
            self.interrupted = Some(InterruptedMember::NotAttempted(extract_entry_location(
                self.destination,
                &path,
            )));
            return Err(error);
        }
        let outpath = self.resolver.resolve(&self.directory, &path)?;
        if self.first_name.is_none() {
            self.first_name = outpath
                .components()
                .next()
                .map(|component| component.as_os_str().to_string_lossy().into_owned());
        }
        let created = match content {
            MemberContent::Directory => {
                self.directory.create_directories(&outpath)?;
                outpath
            }
            MemberContent::File(reader) => {
                let (mut file, created) = self.directory.create_file(&outpath)?;
                if let Err(error) = copy_with_big_buf(reader, &mut file, self.cancelled) {
                    drop(file);
                    let removed = self.directory.remove_file(&created);
                    if error == ArchiveError::Cancelled {
                        let location = extract_entry_location(self.destination, &created);
                        self.interrupted = Some(if removed.is_ok() {
                            InterruptedMember::NotAttempted(location)
                        } else {
                            InterruptedMember::Failed(location)
                        });
                    }
                    return Err(error);
                }
                created
            }
        };
        self.completed
            .push(extract_entry_location(self.destination, &created));
        self.progress.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Remaining locations exclude any member already passed to `extract_member`.
    /// They are best-effort decoder metadata, not a request to read more content.
    pub(super) fn finish(
        self,
        result: Result<(), ArchiveError>,
        remaining: impl FnOnce() -> Vec<Location>,
    ) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
        match result {
            Ok(()) => Ok(ArchiveOutcome::Completed(self.first_name)),
            Err(ArchiveError::Cancelled) => {
                let mut failed = Vec::new();
                let mut not_attempted = Vec::new();
                match self.interrupted {
                    Some(InterruptedMember::NotAttempted(location)) => not_attempted.push(location),
                    Some(InterruptedMember::Failed(location)) => failed.push(location),
                    None => {}
                }
                not_attempted.extend(remaining());
                Ok(ArchiveOutcome::Cancelled {
                    completed: self.completed,
                    failed,
                    not_attempted,
                })
            }
            Err(error) => Err(error),
        }
    }
}

pub(super) fn extract_entry_location(destination: &Path, relative: &Path) -> Location {
    Location::local(destination.join(relative))
}
