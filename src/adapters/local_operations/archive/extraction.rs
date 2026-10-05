// SPDX-License-Identifier: MIT

//! Per-operation extraction policy, independent of archive decoding libraries.
//!
//! Decoders lend member streams to the session and supply already-known pending
//! names only on cancellation. The session validates and maps destination reports
//! without scanning ahead, probing the filesystem or reserving pending names.
//!
//! Members are written into a hidden `.strata-extraction-<uuid>` folder created
//! under the destination on the first member, so names are resolved against an
//! empty folder. [`ExtractionSession::finish`] publishes that folder for every
//! outcome; dropping a session without `finish`, as a decoder panic does,
//! publishes it as a failed extraction.
use std::{
    collections::HashSet,
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::model::Location;

use super::{
    ArchiveError, COPY_BUF, archive_failed, check_archive_cancelled,
    destination::{ExtractNameResolver, ExtractionDestination, sanitized_archive_path},
};

#[cfg(test)]
mod tests;

pub(super) enum MemberContent<'a> {
    Directory,
    File(&'a mut dyn Read, Option<u64>),
}

/// Result of an extract that may stop after writing some members.
#[derive(Debug)]
pub(super) enum ArchiveOutcome<T> {
    Completed(T),
    Cancelled {
        completed: Vec<Location>,
        failed: Vec<Location>,
        not_attempted: Vec<Location>,
    },
}

/// Member paths are relative to the staging folder.
enum InterruptedMember {
    NotAttempted(PathBuf),
    Failed(PathBuf),
}

struct Staging {
    name: OsString,
    directory: ExtractionDestination,
}

pub(super) struct ExtractionSession<'a> {
    destination: &'a Path,
    archive_name: &'a str,
    directory: ExtractionDestination,
    staging: Option<Staging>,
    resolver: ExtractNameResolver,
    progress: &'a AtomicUsize,
    cancelled: &'a AtomicBool,
    roots: Vec<PathBuf>,
    seen_roots: HashSet<PathBuf>,
    completed: Vec<PathBuf>,
    /// Whether a file member completed or a partial file could not be removed.
    /// Staging that holds only directories is not worth keeping after a
    /// failure or cancellation.
    has_content: bool,
    interrupted: Option<InterruptedMember>,
    written: u64,
    available_bytes: Option<u64>,
}

impl<'a> ExtractionSession<'a> {
    pub(super) fn open(
        destination: &'a Path,
        archive_name: &'a str,
        progress: &'a AtomicUsize,
        cancelled: &'a AtomicBool,
    ) -> Result<Self, ArchiveError> {
        let directory = ExtractionDestination::open(destination)?;
        let available_bytes = directory.available_bytes()?;
        Ok(Self::from_open(
            destination,
            archive_name,
            directory,
            progress,
            cancelled,
            available_bytes,
        ))
    }

    fn from_open(
        destination: &'a Path,
        archive_name: &'a str,
        directory: ExtractionDestination,
        progress: &'a AtomicUsize,
        cancelled: &'a AtomicBool,
        available_bytes: Option<u64>,
    ) -> Self {
        Self {
            destination,
            archive_name,
            directory,
            staging: None,
            resolver: ExtractNameResolver::new(),
            progress,
            cancelled,
            roots: Vec::new(),
            seen_roots: HashSet::new(),
            completed: Vec::new(),
            has_content: false,
            interrupted: None,
            written: 0,
            available_bytes,
        }
    }

    #[cfg(test)]
    pub(super) fn open_with_available_bytes(
        destination: &'a Path,
        archive_name: &'a str,
        progress: &'a AtomicUsize,
        cancelled: &'a AtomicBool,
        available_bytes: Option<u64>,
    ) -> Result<Self, ArchiveError> {
        Ok(Self::from_open(
            destination,
            archive_name,
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

    /// Sequential formats that cannot cheaply sum headers rely on per-member checks.
    pub(super) fn preflight_claimed_size(&self, claimed: u128) -> Result<(), ArchiveError> {
        match self.remaining() {
            Some(available) if claimed > u128::from(available) => Err(archive_failed(format!(
                "Archive declared size ({claimed} bytes) exceeds the {available} bytes of free space at the destination"
            ))),
            _ => Ok(()),
        }
    }

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

    fn ensure_staging(&mut self) -> Result<(), ArchiveError> {
        if self.staging.is_none() {
            let (name, directory) = self.directory.create_staging()?;
            self.staging = Some(Staging { name, directory });
        }
        Ok(())
    }

    /// Processes one member. On error the decoder must stop and call `finish`.
    /// Names retain the adapters' legacy string conversion until decoder evaluation.
    pub(super) fn extract_member(
        &mut self,
        name: &str,
        content: MemberContent<'_>,
    ) -> Result<(), ArchiveError> {
        let path = sanitized_archive_path(name)?;
        if let Err(error) = self.check_cancelled() {
            self.interrupted = Some(InterruptedMember::NotAttempted(
                self.resolver.apply_known_rename(&path),
            ));
            return Err(error);
        }
        if let MemberContent::File(_, Some(declared)) = &content {
            self.ensure_member_fits(name, *declared)?;
        }
        self.ensure_staging()?;
        let staging = &self
            .staging
            .as_ref()
            .expect("staging exists after ensure_staging")
            .directory;
        let outpath = self.resolver.resolve(staging, &path)?;
        let created = match content {
            MemberContent::Directory => {
                staging.create_directories(&outpath)?;
                outpath
            }
            content => {
                let (mut file, created) = staging.create_file(&outpath)?;
                let result = match content {
                    MemberContent::File(reader, declared_size) => copy_member(
                        name,
                        reader,
                        &mut file,
                        self.cancelled,
                        declared_size,
                        self.remaining(),
                    ),
                    MemberContent::Directory => unreachable!(),
                };
                match result {
                    Ok(copied) => {
                        self.written = self.written.saturating_add(copied);
                        self.has_content = true;
                        created
                    }
                    Err(error) => {
                        drop(file);
                        let removed = staging.remove_file(&created);
                        if removed.is_err() {
                            self.has_content = true;
                        }
                        if error == ArchiveError::Cancelled {
                            self.interrupted = Some(if removed.is_ok() {
                                InterruptedMember::NotAttempted(created)
                            } else {
                                InterruptedMember::Failed(created)
                            });
                        }
                        return Err(error);
                    }
                }
            }
        };
        if let Some(root) = created.components().next() {
            let root = PathBuf::from(root.as_os_str());
            if self.seen_roots.insert(root.clone()) {
                self.roots.push(root);
            }
        }
        self.completed.push(created);
        self.progress.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Publishes the staging folder for every outcome and reports where the
    /// output landed.
    ///
    /// A completed extraction moves a single top-level entry up under its own
    /// name (suffixed only if the destination already uses it) and renames a
    /// staging folder holding several entries to the archive stem. A failed or
    /// cancelled extraction that wrote anything keeps it under the archive stem
    /// and names that folder; empty staging is removed.
    ///
    /// Pending names exclude members already passed to `extract_member`.
    /// Reports apply established top-level renames, but cannot predict final leaf
    /// conflicts for unattempted members. Invalid names are omitted, not errors.
    pub(super) fn finish(
        mut self,
        result: Result<(), ArchiveError>,
        remaining: impl FnOnce() -> Vec<String>,
    ) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
        // Taking the staging folder disarms the `Drop` publication.
        let staging = self.staging.take();
        let directory = &self.directory;
        let destination = self.destination;
        let archive_name = self.archive_name;
        match result {
            Ok(()) => {
                let Some(staging) = staging else {
                    return Ok(ArchiveOutcome::Completed(None));
                };
                let first_name = match self.roots.as_slice() {
                    [] => None,
                    [root] => Some(
                        directory
                            .publish_single_root(&staging.directory, root)
                            .map_err(|error| staging_kept(&error, &staging))?
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    _ => {
                        return directory
                            .publish_staging_as_folder(&staging.name, archive_name)
                            .map(|name| ArchiveOutcome::Completed(Some(name)))
                            .map_err(|error| staging_kept(&error, &staging));
                    }
                };
                remove_empty(directory, &staging)?;
                Ok(ArchiveOutcome::Completed(first_name))
            }
            Err(ArchiveError::Cancelled) => {
                let kept = keep_or_remove(directory, staging.as_ref(), archive_name, self.has_content)
                    .map_err(ArchiveError::Failed)?;
                let base = |relative: &Path| match &kept {
                    Some(folder) => Location::local(destination.join(folder).join(relative)),
                    None => Location::local(destination.join(relative)),
                };
                let mut completed = Vec::new();
                let mut failed = Vec::new();
                let mut not_attempted = Vec::new();
                // Directories removed with directory-only staging were not kept.
                if kept.is_some() || staging.is_none() {
                    completed.extend(self.completed.iter().map(|path| base(path)));
                } else {
                    not_attempted.extend(self.completed.iter().map(|path| base(path)));
                }
                match self.interrupted.take() {
                    Some(InterruptedMember::NotAttempted(path)) => not_attempted.push(base(&path)),
                    Some(InterruptedMember::Failed(path)) => failed.push(base(&path)),
                    None => {}
                }
                not_attempted.extend(remaining().into_iter().filter_map(|name| {
                    let path = sanitized_archive_path(&name).ok()?;
                    Some(base(&self.resolver.apply_known_rename(&path)))
                }));
                Ok(ArchiveOutcome::Cancelled {
                    completed,
                    failed,
                    not_attempted,
                })
            }
            Err(ArchiveError::Failed(message)) => Err(ArchiveError::Failed(failure_message(
                message,
                keep_or_remove(directory, staging.as_ref(), archive_name, self.has_content),
            ))),
        }
    }
}

impl Drop for ExtractionSession<'_> {
    /// A session dropped before `finish` (a decoder panic) keeps whatever it
    /// wrote under the archive stem, exactly like a failed extraction.
    fn drop(&mut self) {
        if let Some(staging) = self.staging.take() {
            let _ = keep_or_remove(
                &self.directory,
                Some(&staging),
                self.archive_name,
                self.has_content,
            );
        }
    }
}

fn failure_message(message: String, kept: Result<Option<String>, String>) -> String {
    match kept {
        Ok(Some(folder)) => append_sentence(
            &message,
            &format!("Extracted entries remain in `{folder}`."),
        ),
        Ok(None) => message,
        Err(error) => append_sentence(&message, &error),
    }
}

/// After a completed publish the staging folder must be empty.
fn remove_empty(parent: &ExtractionDestination, staging: &Staging) -> Result<(), ArchiveError> {
    match parent.remove_empty_staging(&staging.name) {
        Ok(true) => Ok(()),
        Ok(false) => Err(staging_kept("Some extracted entries were not published", staging)),
        Err(error) => Err(archive_failed(error)),
    }
}

/// Staging without any file, such as the directories left by a member that
/// failed to decrypt, is removed. Anything else is published under the
/// archive stem. Errors name the hidden staging folder, which still holds the
/// output.
fn keep_or_remove(
    parent: &ExtractionDestination,
    staging: Option<&Staging>,
    archive_name: &str,
    has_content: bool,
) -> Result<Option<String>, String> {
    let Some(staging) = staging else {
        return Ok(None);
    };
    let removed = if has_content {
        parent.remove_empty_staging(&staging.name)
    } else {
        parent.remove_directory_only_staging(&staging.name)
    };
    let kept = removed.and_then(|removed| {
        if removed {
            Ok(None)
        } else {
            parent
                .publish_staging_as_folder(&staging.name, archive_name)
                .map(Some)
        }
    });
    kept.map_err(|error| staging_kept_message(&error, staging))
}

fn staging_kept_message(error: &str, staging: &Staging) -> String {
    append_sentence(
        error,
        &format!(
            "Extracted entries remain in `{}`.",
            staging.name.to_string_lossy()
        ),
    )
}

/// Joins `sentence` to `message`, ending `message` with a period first if it
/// has no sentence punctuation. The quoted folder name in `sentence` must keep
/// its backticks: the password-retry check ignores text between them.
fn append_sentence(message: &str, sentence: &str) -> String {
    let separator = if message.ends_with(['.', '!', '?']) {
        " "
    } else {
        ". "
    };
    format!("{message}{separator}{sentence}")
}

fn staging_kept(error: &str, staging: &Staging) -> ArchiveError {
    ArchiveError::Failed(staging_kept_message(error, staging))
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

/// An extra byte past either limit is probed before it can be written.
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
