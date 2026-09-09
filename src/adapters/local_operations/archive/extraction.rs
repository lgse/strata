// SPDX-License-Identifier: MIT

//! Per-operation extraction policy, independent of archive decoding libraries.
//!
//! Decoders lend member streams to the session and supply already-known pending
//! names only on cancellation. The session validates and maps destination reports
//! without scanning ahead, probing the filesystem or reserving pending names.
//! Claimed uncompressed sizes are checked against destination free space before
//! writing, and extracted bytes must match the size declared for each member.
//! Filesystems that do not report capacity skip the free-space checks; the size
//! match still applies.

use std::{
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::model::Location;

use super::{
    ArchiveError, COPY_BUF, archive_failed, check_archive_cancelled,
    destination::{ExtractNameResolver, ExtractionDestination, validated_archive_path},
};

#[cfg(test)]
mod tests;

/// Output classification, not a complete archive entry type. Decoders currently
/// flatten non-directory entries into file streams; this is a provisional boundary.
pub(super) enum MemberContent<'a> {
    Directory,
    /// File stream and the uncompressed size declared by the archive header, if known.
    File(&'a mut dyn Read, Option<u64>),
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
    written: u64,
    /// Free space at open, or `None` when the filesystem does not report it.
    available_bytes: Option<u64>,
}

impl<'a> ExtractionSession<'a> {
    /// Pins `destination` and records its free space for later preflight checks.
    ///
    /// # Errors
    ///
    /// - [`Failed`] if the destination cannot be opened or its free space cannot
    ///   be queried
    ///
    /// [`Failed`]: ArchiveError::Failed
    pub(super) fn open(
        destination: &'a Path,
        progress: &'a AtomicUsize,
        cancelled: &'a AtomicBool,
    ) -> Result<Self, ArchiveError> {
        let directory = ExtractionDestination::open(destination)?;
        let available_bytes = directory.available_bytes()?;
        Ok(Self::from_open(
            destination,
            directory,
            progress,
            cancelled,
            available_bytes,
        ))
    }

    fn from_open(
        destination: &'a Path,
        directory: ExtractionDestination,
        progress: &'a AtomicUsize,
        cancelled: &'a AtomicBool,
        available_bytes: Option<u64>,
    ) -> Self {
        Self {
            destination,
            directory,
            resolver: ExtractNameResolver::new(),
            progress,
            cancelled,
            first_name: None,
            completed: Vec::new(),
            interrupted: None,
            written: 0,
            available_bytes,
        }
    }

    /// Test constructor that substitutes a free-space ceiling for [`fstatvfs`].
    /// `None` models a filesystem that does not report capacity.
    ///
    /// [`fstatvfs`]: rustix::fs::fstatvfs
    #[cfg(test)]
    pub(super) fn open_with_available_bytes(
        destination: &'a Path,
        progress: &'a AtomicUsize,
        cancelled: &'a AtomicBool,
        available_bytes: Option<u64>,
    ) -> Result<Self, ArchiveError> {
        Ok(Self::from_open(
            destination,
            ExtractionDestination::open(destination)?,
            progress,
            cancelled,
            available_bytes,
        ))
    }

    /// Allows a decoder to stop before opening/decrypting another member.
    pub(super) fn check_cancelled(&self) -> Result<(), ArchiveError> {
        check_archive_cancelled(self.cancelled)
    }

    /// Refuses the archive when its claimed uncompressed size exceeds free space.
    ///
    /// Call before extracting members when the format advertises a total. Sequential
    /// formats that cannot cheaply sum headers rely on per-member checks instead.
    /// Passes when the destination filesystem does not report free space.
    ///
    /// # Errors
    ///
    /// - [`Failed`] if `claimed` is larger than the remaining free space
    ///
    /// [`Failed`]: ArchiveError::Failed
    pub(super) fn preflight_claimed_size(&self, claimed: u128) -> Result<(), ArchiveError> {
        match self.remaining() {
            Some(available) if claimed > u128::from(available) => Err(archive_failed(format!(
                "Archive declared size ({claimed} bytes) exceeds the {available} bytes of free space at the destination"
            ))),
            _ => Ok(()),
        }
    }

    /// Free space left for this session, or `None` when it is not reported.
    fn remaining(&self) -> Option<u64> {
        self.available_bytes
            .map(|available| available.saturating_sub(self.written))
    }

    fn ensure_member_fits(&self, name: &str, declared: u64) -> Result<(), ArchiveError> {
        match self.remaining() {
            Some(available) if declared > available => Err(archive_failed(format!(
                "Archive member `{name}` declared {declared} bytes, but only {available} bytes are free at the destination"
            ))),
            _ => Ok(()),
        }
    }

    /// Processes one member. On error the decoder must stop and call `finish`.
    /// Names retain the adapters' legacy string conversion until decoder evaluation.
    /// File members with a declared size are refused if they do not fit in the
    /// remaining free space, and the copy must produce exactly that many bytes.
    ///
    /// # Errors
    ///
    /// - [`Cancelled`] if the operation is cancelled before the member is written
    /// - [`Failed`] if the path is unsafe, the destination cannot be written, the
    ///   declared size exceeds free space, or extracted bytes do not match the
    ///   declared size
    ///
    /// [`Cancelled`]: ArchiveError::Cancelled
    /// [`Failed`]: ArchiveError::Failed
    pub(super) fn extract_member(
        &mut self,
        name: &str,
        content: MemberContent<'_>,
    ) -> Result<(), ArchiveError> {
        let path = validated_archive_path(name)?;
        if let Err(error) = self.check_cancelled() {
            self.interrupted = Some(InterruptedMember::NotAttempted(extract_entry_location(
                self.destination,
                &self.resolver.apply_known_rename(&path),
            )));
            return Err(error);
        }
        if let MemberContent::File(_, Some(declared)) = &content {
            self.ensure_member_fits(name, *declared)?;
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
            MemberContent::File(reader, declared_size) => {
                let (mut file, created) = self.directory.create_file(&outpath)?;
                match copy_member(
                    name,
                    reader,
                    &mut file,
                    self.cancelled,
                    declared_size,
                    self.remaining(),
                ) {
                    Ok(copied) => {
                        self.written = self.written.saturating_add(copied);
                        created
                    }
                    Err(error) => {
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
                }
            }
        };
        self.completed
            .push(extract_entry_location(self.destination, &created));
        self.progress.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Pending names exclude members already passed to `extract_member`.
    /// Reports apply established top-level renames, but cannot predict final leaf
    /// conflicts for unattempted members. Invalid names are omitted, not errors.
    pub(super) fn finish(
        self,
        result: Result<(), ArchiveError>,
        remaining: impl FnOnce() -> Vec<String>,
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
                not_attempted.extend(remaining().into_iter().filter_map(|name| {
                    let path = validated_archive_path(&name).ok()?;
                    Some(extract_entry_location(
                        self.destination,
                        &self.resolver.apply_known_rename(&path),
                    ))
                }));
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

fn extract_entry_location(destination: &Path, relative: &Path) -> Location {
    Location::local(destination.join(relative))
}

fn declared_size_exceeded(name: &str, declared: u64) -> ArchiveError {
    archive_failed(format!(
        "Archive member `{name}` declared {declared} bytes but produced more"
    ))
}

fn declared_size_short(name: &str, declared: u64, actual: u64) -> ArchiveError {
    archive_failed(format!(
        "Archive member `{name}` declared {declared} bytes but produced {actual} bytes"
    ))
}

fn destination_full(name: &str, available: u64) -> ArchiveError {
    archive_failed(format!(
        "Not enough free space at the destination to extract `{name}` ({available} bytes available)"
    ))
}

/// Copies `reader` to `writer`, enforcing declared size and remaining free space.
///
/// Reads at most the declared uncompressed size and the remaining destination
/// capacity, when either is known. An extra byte past either limit, or a short
/// read versus a declared size, fails without leaving the extra data on disk.
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set between reads
/// - [`Failed`] if a read or write fails, extracted bytes do not match `declared_size`,
///   or the remaining free space is exhausted before the stream ends
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn copy_member(
    name: &str,
    mut reader: impl Read,
    writer: &mut (impl Write + ?Sized),
    cancelled: &AtomicBool,
    declared_size: Option<u64>,
    remaining_disk: Option<u64>,
) -> Result<u64, ArchiveError> {
    let mut buf = vec![0u8; COPY_BUF];
    let mut copied = 0u64;
    loop {
        check_archive_cancelled(cancelled)?;
        let declared_remaining = declared_size.map(|declared| declared.saturating_sub(copied));
        let disk_remaining = remaining_disk.map(|disk| disk.saturating_sub(copied));
        let allowed = match (declared_remaining, disk_remaining) {
            (Some(declared), Some(disk)) => declared.min(disk),
            (Some(limit), None) | (None, Some(limit)) => limit,
            (None, None) => u64::MAX,
        };
        if allowed == 0 {
            let mut probe = [0u8; 1];
            if reader.read(&mut probe).map_err(archive_failed)? == 0 {
                break;
            }
            return Err(match declared_size {
                Some(declared) if declared_remaining == Some(0) => {
                    declared_size_exceeded(name, declared)
                }
                _ => destination_full(
                    name,
                    remaining_disk
                        .expect("should know free space when the declared size is not exhausted"),
                ),
            });
        }
        let cap = buf
            .len()
            .min(usize::try_from(allowed).unwrap_or(usize::MAX));
        let n = reader.read(&mut buf[..cap]).map_err(archive_failed)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).map_err(archive_failed)?;
        copied = copied.saturating_add(n as u64);
    }
    if let Some(declared) = declared_size
        && copied != declared
    {
        return Err(declared_size_short(name, declared, copied));
    }
    Ok(copied)
}
