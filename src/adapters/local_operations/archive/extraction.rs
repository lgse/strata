// SPDX-License-Identifier: MIT

//! Per-operation extraction policy, independent of archive decoding libraries.
//!
//! Decoders lend member streams to the session and supply already-known pending
//! names only on cancellation. The session validates and maps destination reports
//! without scanning ahead, probing the filesystem or reserving pending names.
//!
//! Staging isolates conflict naming from the user's files. A password failure
//! discards it, because the retry extracts the whole archive again. Unfinished
//! sessions retain partial output even during decoder panic unwinding.
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    io::{Read, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::SystemTime,
};

use crate::model::Location;

use super::{
    ArchiveError, COPY_BUF, archive_failed, archive_io_failed, archive_read_failed,
    check_archive_cancelled,
    destination::{
        ExtractNameResolver, ExtractionDestination, process_umask, sanitized_archive_path,
    },
};

#[cfg(test)]
mod tests;

/// Linux `PATH_MAX` less its terminating NUL; longer symlink targets are refused.
pub(super) const MAX_SYMLINK_TARGET_BYTES: u64 = 4095;

pub(super) enum MemberContent<'a> {
    Directory,
    File(&'a mut dyn Read, Option<u64>),
    Symlink(&'a [u8]),
    /// Only earlier extracted members may be link targets.
    HardLink(&'a Path),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MemberMetadata {
    pub(super) mode: Option<u32>,
    pub(super) modified: Option<SystemTime>,
}

impl MemberMetadata {
    pub(super) const NONE: Self = Self {
        mode: None,
        modified: None,
    };
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
    /// Directory-only leftovers can be discarded after a failed password attempt.
    has_content: bool,
    interrupted: Option<InterruptedMember>,
    written: u64,
    available_bytes: Option<u64>,
    records_hard_link_targets: bool,
    /// Native archive identity must survive conflict renames for later hard links.
    created_names: HashMap<PathBuf, PathBuf>,
    directories: Vec<(PathBuf, MemberMetadata)>,
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
            records_hard_link_targets: false,
            created_names: HashMap::new(),
            directories: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn open_with_metadata_calls(
        destination: &'a Path,
        archive_name: &'a str,
        progress: &'a AtomicUsize,
        cancelled: &'a AtomicBool,
        calls: super::destination::MetadataCalls,
    ) -> Result<Self, ArchiveError> {
        Ok(Self::from_open(
            destination,
            archive_name,
            ExtractionDestination::open(destination)?.with_metadata_calls(calls),
            progress,
            cancelled,
            None,
        ))
    }

    /// Avoid retaining every member name in formats without hard links.
    pub(super) fn record_hard_link_targets(&mut self) {
        self.records_hard_link_targets = true;
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
            Some(available) if claimed > u128::from(available) => {
                Err(archive_failed(rust_i18n::t!(
                    "Archive declared size (%{claimed} bytes) exceeds the %{available} bytes of free space at the destination",
                    claimed = claimed,
                    available = available
                )))
            }
            _ => Ok(()),
        }
    }

    fn remaining(&self) -> Option<u64> {
        self.available_bytes
            .map(|available| available.saturating_sub(self.written))
    }

    fn ensure_member_fits(&self, name: &str, declared: u64) -> Result<(), ArchiveError> {
        match self.remaining() {
            Some(available) if declared > available => Err(archive_failed(rust_i18n::t!(
                "Archive member `%{name}` declared %{declared} bytes, but only %{available} bytes are free at the destination",
                name = name,
                declared = declared,
                available = available
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

    fn hard_link_target(&self, name: &str, target: &Path) -> Result<PathBuf, ArchiveError> {
        sanitized_archive_path(target)
            .ok()
            .and_then(|path| self.created_names.get(&path))
            .cloned()
            .ok_or_else(|| {
                archive_failed(rust_i18n::t!(
                    "Archive member `%{name}` is a hard link to `%{target}`, which was not extracted",
                    name = name,
                    target = target.display()
                ))
            })
    }

    /// On error the decoder must stop and call `finish`.
    pub(super) fn extract_member(
        &mut self,
        name: impl AsRef<OsStr>,
        content: MemberContent<'_>,
        metadata: MemberMetadata,
    ) -> Result<(), ArchiveError> {
        let path = sanitized_archive_path(&name)?;
        let name = name.as_ref().to_string_lossy();
        if let Err(error) = self.check_cancelled() {
            self.interrupted = Some(InterruptedMember::NotAttempted(
                self.resolver.apply_known_rename(&path),
            ));
            return Err(error);
        }
        if let MemberContent::File(_, Some(declared)) = &content {
            self.ensure_member_fits(&name, *declared)?;
        }
        // Checked before staging exists, so a refused first member leaves nothing behind.
        let hard_link_target = match &content {
            MemberContent::Symlink(target) => {
                validate_link_target(&name, target)?;
                None
            }
            MemberContent::HardLink(target) => Some(self.hard_link_target(&name, target)?),
            _ => None,
        };
        self.ensure_staging()?;
        let staging = &self
            .staging
            .as_ref()
            .expect("staging exists after ensure_staging")
            .directory;
        let outpath = self.resolver.resolve(staging, &path)?;
        let is_directory = matches!(content, MemberContent::Directory);
        let created = match content {
            MemberContent::Directory => {
                staging.create_directories(&outpath)?;
                self.directories.push((outpath.clone(), metadata));
                outpath
            }
            MemberContent::Symlink(target) => {
                staging.create_symlink(&outpath, OsStr::from_bytes(target), metadata.modified)?
            }
            MemberContent::HardLink(_) => staging.create_hard_link(
                &outpath,
                &hard_link_target.expect("hard links are resolved before staging"),
            )?,
            MemberContent::File(reader, declared_size) => {
                let (mut file, created) = staging.create_file(&outpath, metadata.mode)?;
                let result = copy_member(
                    &name,
                    reader,
                    &mut file,
                    self.cancelled,
                    declared_size,
                    self.remaining(),
                )
                .and_then(|copied| match metadata.modified {
                    Some(modified) => staging
                        .set_file_times(&file, modified)
                        .map(|()| copied)
                        .map_err(|error| {
                            archive_failed(rust_i18n::t!(
                                "Could not restore the modification time of `%{path}`: %{error}",
                                path = name,
                                error = crate::services::io_error_message(&error.into())
                            ))
                        }),
                    None => Ok(copied),
                });
                match result {
                    Ok(copied) => {
                        self.written = self.written.saturating_add(copied);
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
        if !is_directory {
            self.has_content = true;
            if self.records_hard_link_targets {
                self.created_names.insert(path, created.clone());
            }
        }
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
                let (top_level, nested): (Vec<_>, Vec<_>) =
                    merge_repeated_directories(std::mem::take(&mut self.directories))
                        .into_iter()
                        .partition(|(path, _)| path.components().count() == 1);
                // Nested directories never move on their own during publication.
                if let Err(error) = restore_directory_metadata(&staging.directory, nested) {
                    let kept =
                        keep_or_remove(directory, Some(&staging), archive_name, self.has_content);
                    return Err(ArchiveError::Failed(failure_message(error, kept)));
                }
                let (name, is_folder) = match self.roots.as_slice() {
                    [] => {
                        remove_empty(directory, &staging)?;
                        return Ok(ArchiveOutcome::Completed(None));
                    }
                    [root] => {
                        let leaf = directory
                            .publish_single_root(&staging.directory, root)
                            .map_err(|error| staging_kept(&error, &staging))?;
                        remove_empty(directory, &staging)?;
                        (leaf, false)
                    }
                    _ => {
                        let folder = directory
                            .publish_staging_as_folder(&staging.name, archive_name)
                            .map_err(|error| staging_kept(&error, &staging))?;
                        (OsString::from(folder), true)
                    }
                };
                // Cross-parent directory moves may need owner write access to `..`.
                let published = top_level
                    .into_iter()
                    .map(|(path, metadata)| {
                        let path = if is_folder {
                            Path::new(&name).join(path)
                        } else {
                            PathBuf::from(&name)
                        };
                        (path, metadata)
                    })
                    .collect();
                let name = name.to_string_lossy().into_owned();
                restore_directory_metadata(directory, published).map_err(|error| {
                    ArchiveError::Failed(append_sentence(&error, &entries_remain_in(&name)))
                })?;
                Ok(ArchiveOutcome::Completed(Some(name)))
            }
            Err(ArchiveError::Cancelled) => {
                let kept =
                    keep_or_remove(directory, staging.as_ref(), archive_name, self.has_content)
                        .map_err(ArchiveError::Failed)?;
                let base = |relative: &Path| match &kept {
                    Some(folder) => Location::local(destination.join(folder).join(relative)),
                    None => Location::local(destination.join(relative)),
                };
                let mut completed = Vec::new();
                let mut failed = Vec::new();
                let mut not_attempted = Vec::new();
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
            Err(
                error @ (ArchiveError::PasswordRequired(_) | ArchiveError::IncorrectPassword(_)),
            ) => {
                let discarded = staging
                    .as_ref()
                    .map_or(Ok(()), |staging| directory.remove_staging(&staging.name));
                // Output that cannot be discarded would make the retry take a
                // numbered name, so report an ordinary failure instead.
                match discarded {
                    Ok(()) => Err(error),
                    Err(removal) => Err(ArchiveError::Failed(failure_message(
                        append_sentence(&error.to_string(), &removal),
                        keep_or_remove(directory, staging.as_ref(), archive_name, self.has_content),
                    ))),
                }
            }
            Err(ArchiveError::Failed(message)) => Err(ArchiveError::Failed(failure_message(
                message,
                keep_or_remove(directory, staging.as_ref(), archive_name, self.has_content),
            ))),
        }
    }
}

impl Drop for ExtractionSession<'_> {
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
        Ok(Some(folder)) => append_sentence(&message, &entries_remain_in(&folder)),
        Ok(None) => message,
        Err(error) => append_sentence(&message, &error),
    }
}

/// Restore each directory once: an earlier restrictive mode could block a later restore.
fn merge_repeated_directories(
    directories: Vec<(PathBuf, MemberMetadata)>,
) -> Vec<(PathBuf, MemberMetadata)> {
    let mut positions: HashMap<PathBuf, usize> = HashMap::new();
    let mut merged: Vec<(PathBuf, MemberMetadata)> = Vec::new();
    for (path, metadata) in directories {
        match positions.get(&path) {
            Some(&position) => {
                let earlier = &mut merged[position].1;
                *earlier = MemberMetadata {
                    mode: metadata.mode.or(earlier.mode),
                    modified: metadata.modified.or(earlier.modified),
                };
            }
            None => {
                positions.insert(path.clone(), merged.len());
                merged.push((path, metadata));
            }
        }
    }
    merged.retain(|(_, metadata)| *metadata != MemberMetadata::NONE);
    merged
}

/// Deepest-first restoration keeps ancestors traversable; rollback reverses that order.
fn restore_directory_metadata(
    destination: &ExtractionDestination,
    mut directories: Vec<(PathBuf, MemberMetadata)>,
) -> Result<(), String> {
    directories.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    let umask = process_umask();
    for (index, (path, metadata)) in directories.iter().enumerate() {
        if let Err(error) = destination.apply_directory_metadata(path, *metadata, umask) {
            for (path, _) in directories[..=index].iter().rev() {
                destination.reset_directory_mode(path, umask);
            }
            return Err(error);
        }
    }
    Ok(())
}

fn validate_link_target(name: &str, target: &[u8]) -> Result<(), ArchiveError> {
    if target.is_empty() || target.len() as u64 > MAX_SYMLINK_TARGET_BYTES || target.contains(&0) {
        return Err(archive_failed(rust_i18n::t!(
            "Archive member `%{name}` has an invalid symbolic link target",
            name = name
        )));
    }
    Ok(())
}

fn remove_empty(parent: &ExtractionDestination, staging: &Staging) -> Result<(), ArchiveError> {
    match parent.remove_empty_staging(&staging.name) {
        Ok(true) => Ok(()),
        Ok(false) => Err(staging_kept(
            &crate::i18n::tr("Some extracted entries were not published"),
            staging,
        )),
        Err(error) => Err(archive_failed(error)),
    }
}

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
    append_sentence(error, &entries_remain_in(&staging.name.to_string_lossy()))
}

fn entries_remain_in(folder: &str) -> String {
    rust_i18n::t!("Extracted entries remain in `%{folder}`.", folder = folder).into_owned()
}

fn append_sentence(message: &str, sentence: &str) -> String {
    let separator = if message.ends_with('。') {
        ""
    } else if message.ends_with(['.', '!', '?']) {
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
    archive_failed(rust_i18n::t!(
        "Archive member `%{name}` declared %{declared} bytes but produced more",
        name = name,
        declared = declared
    ))
}

fn declared_size_short(name: &str, declared: u64, actual: u64) -> ArchiveError {
    archive_failed(rust_i18n::t!(
        "Archive member `%{name}` declared %{declared} bytes but produced %{actual} bytes",
        name = name,
        declared = declared,
        actual = actual
    ))
}

fn destination_full(name: &str, available: u64) -> ArchiveError {
    archive_failed(rust_i18n::t!(
        "Not enough free space at the destination to extract `%{name}` (%{available} bytes available)",
        name = name,
        available = available
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
            if reader.read(&mut probe).map_err(archive_read_failed)? == 0 {
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
        let n = reader.read(&mut buf[..cap]).map_err(archive_read_failed)?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .map_err(|error| archive_io_failed(&error))?;
        copied = copied.saturating_add(n as u64);
    }
    if let Some(declared) = declared_size
        && copied != declared
    {
        return Err(declared_size_short(name, declared, copied));
    }
    Ok(copied)
}
