// SPDX-License-Identifier: MIT

//! Staged archive publication and the existing format-specific writers.

#[cfg(test)]
mod tests;

use super::super::{
    copy_engine,
    fs_ops::{Entry, EntryStat, OffMain, fs},
    mount_profile::{self, known_mount, name_limit, name_rules},
    names::{fix::split_extension, numbered_name},
    native_fs::{
        LocalFileIdentity, Pinned, Recorded, local_directory_children, open_local_child_directory,
        open_local_parent_directory,
    },
    owed::{Deletion, Owed},
    placement::{
        Displacement, Exchanged, NUMBERED_NAME_ATTEMPTS, PlaceError, RenameFailure,
        displacement_without_trash, exchange_through, file_creation_error, place_staged, put_back,
        remove_if_unchanged, reserve_hidden_name, set_aside, set_aside_if_safe,
    },
    placement_reason,
    progress::TransferProgressTracker,
    replacement::{DisplacedOriginal, cannot_replace_safely, original_kept_aside},
    trash::trash_verified,
    typed_names,
};
use super::{
    ArchiveError, COPY_BUF, archive_failed, check_archive_cancelled, copy_with_big_buf,
    decoders::FILE_ATTRIBUTE_UNIX_EXTENSION, destination::process_umask,
};
use crate::{
    adapters::volume::Family,
    model::hidden_names::HiddenName,
    services::{ItemIdentity, PlacedSnapshot, ReplacedOriginal, TransferConflict, error_detail},
};
use gtk::{gio, glib};
use rustix::fs::AtFlags;
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::fs::PermissionsExt,
    },
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Writes an archive through a private staging file, then publishes it at
/// `archive_path` without ever replacing an entry it did not account for.
///
/// Staging, encoding and publication run in one blocking step. A
/// replacement strategy is chosen before encoding, so an archive is never
/// written that cannot be published: the original goes to Trash where
/// possible, otherwise it is exchanged or set aside and deleted once the new
/// archive is on disk. [`KeepBoth`] numbers the name without encoding again.
/// The published mode is the replaced file's, or `0o666` masked by the umask.
/// [`Failed`]: ArchiveError::Failed
/// [`KeepBoth`]: TransferConflict::KeepBoth
pub(super) async fn write_staged_archive<F>(
    destination: &Path,
    archive_path: &Path,
    conflict: TransferConflict,
    cancelled: Arc<AtomicBool>,
    progress: Option<&TransferProgressTracker>,
    write_archive: F,
) -> Result<(String, Option<ReplacedOriginal>, Option<PlacedSnapshot>), ArchiveError>
where
    F: FnOnce(std::fs::File) -> Result<(), ArchiveError> + Send + 'static,
{
    let name = archive_path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| crate::i18n::tr("Archive filename must be UTF-8"))?
        .to_owned();
    let barrier = match conflict {
        TransferConflict::ReplaceExisting => Some(
            copy_engine::barrier(destination.to_path_buf())
                .await
                .map_err(|error| archive_failed(error.message()))?,
        ),
        _ => None,
    };
    let job = Publication {
        destination: destination.to_path_buf(),
        name: name.clone(),
        sync_data: barrier
            .as_ref()
            .is_some_and(copy_engine::Barrier::syncs_each_file),
    };
    let published = gio::spawn_blocking(move || job.write(conflict, &cancelled, write_archive))
        .await
        .map_err(|_| archive_failed(crate::i18n::tr("Compression stopped unexpectedly")))??;
    let (original, placed) = match published.displaced {
        Displaced::None => (None, None),
        Displaced::Trashed(original, placed) => (Some(ReplacedOriginal::Trashed(original)), placed),
        Displaced::Aside(aside) => {
            let mut owed = Owed::default();
            owed.barrier = barrier;
            owed.owe(Deletion::Displaced(aside, destination.join(&name)));
            owed.settle(progress, &gio::Cancellable::new())
                .await
                .map_err(|error| archive_failed(error.message()))?;
            (Some(ReplacedOriginal::Deleted), None)
        }
    };
    Ok((published.name, original, placed))
}

enum Strategy {
    New,
    Numbered,
    /// Replaces only the item seen before encoding.
    Replace(Replacement, Target),
}

struct Target {
    recorded: EntryStat,
    pin: Pinned,
}

#[derive(Clone, Copy)]
enum Replacement {
    Trash,
    Exchange,
    Aside,
}

enum Displaced {
    None,
    Trashed(Option<ItemIdentity>, Option<PlacedSnapshot>),
    Aside(DisplacedOriginal),
}

struct Published {
    name: String,
    displaced: Displaced,
}

struct Stage {
    ownership: OnceLock<copy_engine::Ownership>,
    stale_written_times: bool,
    parent: Arc<OwnedFd>,
    name: OsString,
    identity: LocalFileIdentity,
}

impl Stage {
    fn entry(&self) -> Entry<'_> {
        Entry::new(&*self.parent, &self.name)
    }

    fn place(&self, name: &OsStr) -> Result<(), PlaceError> {
        place_staged(
            fs(),
            self.entry(),
            Entry::new(&*self.parent, name),
            self.identity,
        )
        .map(drop)
    }
}

struct Publication {
    destination: PathBuf,
    name: String,
    sync_data: bool,
}

impl Publication {
    fn write<F>(
        &self,
        conflict: TransferConflict,
        cancelled: &AtomicBool,
        write_archive: F,
    ) -> Result<Published, ArchiveError>
    where
        F: FnOnce(std::fs::File) -> Result<(), ArchiveError>,
    {
        let parent = Arc::new(
            open_local_parent_directory(&self.destination)
                .map_err(|error| self.creation_failed(error))?,
        );
        if let Some(message) =
            typed_names::refused(&self.name, &name_rules(fs(), parent.as_fd()), false)
        {
            return Err(archive_failed(message));
        }
        let (mode, strategy) = match conflict {
            TransferConflict::FailIfExists | TransferConflict::Merge => {
                (default_file_mode(), Strategy::New)
            }
            TransferConflict::KeepBoth => (default_file_mode(), Strategy::Numbered),
            TransferConflict::ReplaceExisting => match self.existing(&parent)? {
                None => (default_file_mode(), Strategy::New),
                Some(stat) => {
                    let mode = if stat.kind == rustix::fs::FileType::RegularFile {
                        stat.mode
                    } else {
                        default_file_mode()
                    };
                    let replacement = if fs().can_trash(&self.destination.join(&self.name)) {
                        Replacement::Trash
                    } else {
                        self.without_trash(&parent)?
                    };
                    let pin = Pinned::at(
                        fs(),
                        Entry::new(&*parent, OsStr::new(&self.name)),
                        &stat,
                        known_mount(fs(), parent.as_fd()).1.identity,
                    );
                    let target = Target {
                        recorded: stat,
                        pin,
                    };
                    (mode, Strategy::Replace(replacement, target))
                }
            },
        };

        let (name, file) = reserve_hidden_name(HiddenName::Copy, |name| {
            fs().create_exclusive(
                Entry::new(&*parent, name),
                rustix::fs::Mode::from_raw_mode(0o600),
            )
            .map_err(file_creation_error)
        })
        .map_err(|error| self.creation_failed(placement_reason(error)))?;
        let (stage, _pinned) = match fs().stat_fd(file.as_fd()) {
            Ok(stat) => {
                let capabilities = known_mount(fs(), parent.as_fd()).1;
                let pinned = Pinned::at(
                    fs(),
                    Entry::new(&*parent, &name),
                    &stat,
                    capabilities.identity,
                );
                let stage = Stage {
                    ownership: OnceLock::new(),
                    stale_written_times: capabilities.family == Some(Family::Smb),
                    parent,
                    name,
                    identity: stat.identity,
                };
                (stage, pinned)
            }
            Err(error) => return Err(self.creation_failed(errno_text(error))),
        };
        let result =
            self.encode_and_publish(&stage, file, mode, cancelled, strategy, write_archive);
        if result.is_err() {
            let attributes = stage
                .ownership
                .get()
                .and_then(copy_engine::Ownership::attributes);
            remove_if_unchanged(
                fs(),
                stage.entry(),
                stage.identity,
                attributes,
                AtFlags::empty(),
            );
        }
        result
    }

    fn encode_and_publish<F>(
        &self,
        stage: &Stage,
        file: OwnedFd,
        mode: u32,
        cancelled: &AtomicBool,
        strategy: Strategy,
        write_archive: F,
    ) -> Result<Published, ArchiveError>
    where
        F: FnOnce(std::fs::File) -> Result<(), ArchiveError>,
    {
        let handle = fs()
            .dup(file.as_fd())
            .map_err(|error| self.creation_failed(errno_text(error)))?;
        write_archive(std::fs::File::from(file))?;
        check_archive_cancelled(cancelled)?;
        fs().chmod_fd(handle.as_fd(), mode)
            .map_err(|error| self.creation_failed(errno_text(error)))?;
        if self.sync_data {
            fs().sync_data(handle.as_fd())
                .map_err(|error| self.creation_failed(errno_text(error)))?;
        }
        let stat = fs()
            .stat_fd(handle.as_fd())
            .map_err(|error| self.creation_failed(errno_text(error)))?;
        // While it holds a lease, the SMB client shows the server's time only to a forced stat.
        let stat = stage
            .stale_written_times
            .then(|| fs().stat(stage.entry(), true))
            .and_then(Result::ok)
            .filter(|named| named.identity == stat.identity)
            .unwrap_or(stat);
        let _ = stage.ownership.set(copy_engine::file_ownership(&stat));
        // Close before publication so NFS cannot silly-rename a failed stage.
        fs().close(handle)
            .map_err(|error| self.creation_failed(errno_text(error)))?;
        check_archive_cancelled(cancelled)?;
        self.publish(stage, strategy, cancelled)
    }

    fn existing(&self, parent: &OwnedFd) -> Result<Option<EntryStat>, ArchiveError> {
        match fs().stat(Entry::new(parent, OsStr::new(&self.name)), false) {
            Ok(stat) if stat.is_directory() => Err(archive_failed(crate::i18n::tr(
                "An archive cannot replace a folder",
            ))),
            Ok(stat) => Ok(Some(stat)),
            Err(rustix::io::Errno::NOENT) => Ok(None),
            Err(error) => Err(self.creation_failed(errno_text(error))),
        }
    }

    fn without_trash(&self, parent: &OwnedFd) -> Result<Replacement, ArchiveError> {
        match displacement_without_trash(fs(), parent.as_fd(), false) {
            Ok(Some(Displacement::Exchange)) => Ok(Replacement::Exchange),
            Ok(Some(Displacement::SetAside)) => Ok(Replacement::Aside),
            Ok(None) => Err(self.cannot_replace_safely()),
            Err(error) => Err(self.creation_failed(placement_reason(error))),
        }
    }

    fn publish(
        &self,
        stage: &Stage,
        strategy: Strategy,
        cancelled: &AtomicBool,
    ) -> Result<Published, ArchiveError> {
        let (replacement, target) = match strategy {
            Strategy::New => {
                return stage
                    .place(OsStr::new(&self.name))
                    .map(|()| self.published(Displaced::None))
                    .map_err(|error| self.creation_failed(placement_reason(error)));
            }
            Strategy::Numbered => return self.publish_numbered(stage),
            Strategy::Replace(replacement, target) => (replacement, target),
        };
        let Some(stat) = self.existing(&stage.parent)? else {
            return self.publish(stage, Strategy::New, cancelled);
        };
        if !target.pin.unchanged(
            fs(),
            mount_profile::identity_at(fs(), stage.parent.as_fd()),
            &target.recorded,
            &stat,
        ) {
            return Err(archive_failed(crate::i18n::tr(
                "The target changed while the operation was in progress",
            )));
        }
        match replacement {
            Replacement::Trash => self.publish_after_trash(stage, target, &stat, cancelled),
            Replacement::Exchange => self.publish_by_exchange(stage, &stat),
            Replacement::Aside => self.publish_after_set_aside(stage, &stat),
        }
    }

    fn publish_by_exchange(
        &self,
        stage: &Stage,
        current: &EntryStat,
    ) -> Result<Published, ArchiveError> {
        let target = Entry::new(&*stage.parent, OsStr::new(&self.name));
        let attributes = stage
            .ownership
            .get()
            .and_then(copy_engine::Ownership::attributes);
        match exchange_through(fs(), stage.entry(), target, stage.identity, attributes) {
            Ok(Exchanged::Swapped(originals)) => {
                Ok(self.published(Displaced::Aside(DisplacedOriginal {
                    parent: OffMain::new(stage.parent.clone()),
                    originals,
                    name: OsString::from(&self.name),
                    identity: Recorded::of(current),
                })))
            }
            Ok(Exchanged::Refused) => match set_aside_if_safe(fs(), stage.parent.as_fd(), false) {
                Ok(Some(_)) => self.publish_after_set_aside(stage, current),
                Ok(None) => Err(self.cannot_replace_safely()),
                Err(error) => Err(self.creation_failed(placement_reason(error))),
            },
            Err(error) => Err(self.creation_failed(placement_reason(error))),
        }
    }

    fn publish_after_trash(
        &self,
        stage: &Stage,
        target: Target,
        current: &EntryStat,
        cancelled: &AtomicBool,
    ) -> Result<Published, ArchiveError> {
        let trashed = match trash_verified(
            Entry::new(&*stage.parent, OsStr::new(&self.name)),
            &self.destination.join(&self.name),
            current,
            &gio::Cancellable::new(),
        ) {
            Ok(trashed) => trashed,
            Err(error) if error.is_unsupported() => {
                let replacement = self.without_trash(&stage.parent)?;
                return self.publish(stage, Strategy::Replace(replacement, target), cancelled);
            }
            Err(error) => {
                return Err(archive_failed(rust_i18n::t!(
                    "Could not move %{name} to Trash: %{error}",
                    name = self.name,
                    error = crate::services::gio_error_detail(&error.error)
                )));
            }
        };
        stage
            .place(OsStr::new(&self.name))
            .map(|()| {
                let name = OsStr::new(&self.name);
                let placed = stage.ownership.get().and_then(|ownership| {
                    copy_engine::placed_at_proven(
                        fs(),
                        (stage.parent.as_fd(), name),
                        ownership,
                        cancelled,
                    )
                });
                self.published(Displaced::Trashed(trashed, placed))
            })
            .map_err(|error| {
                archive_failed(rust_i18n::t!(
                    "Could not publish the archive; the original is in Trash: %{error}",
                    error = error_detail(placement_reason(error))
                ))
            })
    }

    fn publish_after_set_aside(
        &self,
        stage: &Stage,
        current: &EntryStat,
    ) -> Result<Published, ArchiveError> {
        let target = OsStr::new(&self.name);
        let aside = match set_aside(
            fs(),
            Entry::new(&*stage.parent, target),
            current.identity,
            current.kept_attributes(),
        ) {
            Ok(aside) => aside,
            Err(RenameFailure::Place(PlaceError::Unsupported)) => {
                return Err(self.cannot_replace_safely());
            }
            Err(RenameFailure::Place(error)) => {
                return Err(self.creation_failed(placement_reason(error)));
            }
            Err(RenameFailure::Stranded { temporary, error }) => {
                return Err(archive_failed(original_kept_aside(
                    &self.name,
                    &temporary,
                    &error_detail(placement_reason(error)),
                )));
            }
        };
        match stage.place(target) {
            Ok(()) => Ok(self.published(Displaced::Aside(DisplacedOriginal::aside(
                stage.parent.clone(),
                aside,
            )))),
            Err(error) => match put_back(fs(), stage.parent.as_fd(), &aside, target) {
                Ok(()) => Err(self.creation_failed(placement_reason(error))),
                Err(_) => Err(archive_failed(original_kept_aside(
                    &self.name,
                    &aside.kept(),
                    &error_detail(placement_reason(error)),
                ))),
            },
        }
    }

    fn publish_numbered(&self, stage: &Stage) -> Result<Published, ArchiveError> {
        let (stem, extension) = split_extension(OsStr::new(&self.name), false);
        let mut limit = None;
        for suffix in 0..NUMBERED_NAME_ATTEMPTS {
            let candidate = if suffix == 0 {
                self.name.clone()
            } else if extension.is_empty() {
                return Err(
                    crate::i18n::tr("Archive filename must have a format extension").into(),
                );
            } else {
                let limit = *limit.get_or_insert_with(|| name_limit(fs(), stage.parent.as_fd()));
                numbered_name(stem, suffix, extension, limit, false)
                    .to_string_lossy()
                    .into_owned()
            };
            match stage.place(OsStr::new(&candidate)) {
                Ok(()) => {
                    return Ok(Published {
                        name: candidate,
                        displaced: Displaced::None,
                    });
                }
                Err(PlaceError::Exists) => {}
                Err(error) => return Err(self.creation_failed(placement_reason(error))),
            }
        }
        Err(archive_failed(crate::i18n::tr(
            "No available archive filename",
        )))
    }

    fn published(&self, displaced: Displaced) -> Published {
        Published {
            name: self.name.clone(),
            displaced,
        }
    }

    fn creation_failed(&self, reason: String) -> ArchiveError {
        archive_failed(rust_i18n::t!(
            "Could not create the archive in %{folder}: %{error}",
            folder = self.destination.display(),
            error = error_detail(reason)
        ))
    }

    fn cannot_replace_safely(&self) -> ArchiveError {
        archive_failed(cannot_replace_safely(&self.name))
    }
}

fn default_file_mode() -> u32 {
    0o666 & !process_umask()
}

fn io_text(error: &io::Error) -> String {
    crate::services::io_error_message(error)
}

fn errno_text(error: rustix::io::Errno) -> String {
    io_text(&error.into())
}

fn zip_text(error: zip::result::ZipError) -> String {
    match error {
        zip::result::ZipError::Io(error) => io_text(&error),
        error => encoder_failure(&error),
    }
}

fn sevenz_text(error: sevenz_rust2::Error) -> String {
    match error {
        sevenz_rust2::Error::Io(error, _) | sevenz_rust2::Error::FileOpen(error, _) => {
            io_text(&error)
        }
        error => encoder_failure(&error),
    }
}

/// Encoder diagnostics are untranslated library text, so they are only logged.
fn encoder_failure(error: &dyn std::fmt::Display) -> String {
    tracing::warn!(%error, "archive encoder failed");
    crate::i18n::tr("The archive could not be written")
}

/// An opened compression source, re-read from disk relative to its parent
/// directory rather than trusted from any earlier listing.
enum ArchiveSource {
    /// Open file description for a regular file, opened with `BENEATH`,
    /// `NO_SYMLINKS`, and `NO_MAGICLINKS`.
    File(std::fs::File),
    /// Open directory used to walk children descriptor-relative.
    Directory(std::fs::File),
    Symlink {
        target: PathBuf,
        modified: Option<SystemTime>,
    },
}

fn source_metadata(source: &ArchiveSource) -> Result<(Option<SystemTime>, Option<u32>), String> {
    match source {
        ArchiveSource::File(file) | ArchiveSource::Directory(file) => {
            let metadata = file.metadata().map_err(|error| io_text(&error))?;
            Ok((
                metadata.modified().ok(),
                Some(metadata.permissions().mode()),
            ))
        }
        ArchiveSource::Symlink { modified, .. } => Ok((*modified, None)),
    }
}

fn unix_seconds(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|since_epoch| since_epoch.as_secs())
}

/// DOS timestamps use local time and can only represent 1980–2107.
fn zip_datetime(modified: Option<SystemTime>) -> zip::DateTime {
    let Some(local) = modified
        .and_then(unix_seconds)
        .and_then(|seconds| i64::try_from(seconds).ok())
        .and_then(|seconds| glib::DateTime::from_unix_local(seconds).ok())
    else {
        return zip::DateTime::DEFAULT;
    };
    let part = |value: i32| u8::try_from(value).unwrap_or_default();
    match u16::try_from(local.year()).unwrap_or_default() {
        ..1980 => zip::DateTime::DEFAULT,
        2108.. => zip::DateTime::from_date_and_time(2107, 12, 31, 23, 59, 58)
            .unwrap_or(zip::DateTime::DEFAULT),
        year => zip::DateTime::from_date_and_time(
            year,
            part(local.month()),
            part(local.day_of_month()),
            part(local.hour()),
            part(local.minute()),
            part(local.second()),
        )
        .unwrap_or(zip::DateTime::DEFAULT),
    }
}

fn zip_member_options<'k, 'n>(
    base: zip::write::FileOptions<'k, 'n, ()>,
    modified: Option<SystemTime>,
    mode: Option<u32>,
) -> Result<zip::write::FullFileOptions<'k, 'n>, String> {
    let mut options = base
        .last_modified_time(zip_datetime(modified))
        .into_full_options();
    if let Some(mode) = mode {
        options = options.unix_permissions(mode);
    }
    if let Some(seconds) = modified
        .and_then(unix_seconds)
        .and_then(|seconds| i32::try_from(seconds).ok())
    {
        // Flags byte 1: only the modification time follows.
        let mut field = vec![1];
        field.extend_from_slice(&seconds.to_le_bytes());
        options
            .add_extra_field(0x5455, field, false)
            .map_err(zip_text)?;
    }
    Ok(options)
}

/// Opens the child named `name` inside `parent` without following symbolic links.
///
/// Regular files are opened with [`rustix::fs::ResolveFlags::BENEATH`],
/// [`rustix::fs::ResolveFlags::NO_SYMLINKS`], and
/// [`rustix::fs::ResolveFlags::NO_MAGICLINKS`]. Directories go through
/// [`open_local_child_directory`]. Symlinks are read with `readlinkat` and
/// stored as [`ArchiveSource::Symlink`].
///
/// # Errors
///
/// Returns an error if `name` cannot be inspected, is an unsupported file
/// type, or changes from a regular file between `statat` and `openat2`.
fn open_archive_source<Fd: AsFd>(parent: &Fd, name: &OsStr) -> Result<ArchiveSource, String> {
    let stat = fs()
        .stat(Entry::new(parent, name), false)
        .map_err(errno_text)?;
    match stat.kind {
        rustix::fs::FileType::Symlink => {
            let target = fs()
                .readlink(Entry::new(parent, name))
                .map_err(errno_text)?;
            let modified = u64::try_from(stat.mtime.0)
                .ok()
                .zip(Some(stat.mtime.1))
                .and_then(|(seconds, nanos)| UNIX_EPOCH.checked_add(Duration::new(seconds, nanos)));
            Ok(ArchiveSource::Symlink {
                target: PathBuf::from(target),
                modified,
            })
        }
        rustix::fs::FileType::Directory => open_local_child_directory(parent, name)
            .map(std::fs::File::from)
            .map(ArchiveSource::Directory),
        rustix::fs::FileType::RegularFile => {
            let file = fs()
                .open_at(
                    Entry::new(parent, name),
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::NONBLOCK
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                    Some(
                        rustix::fs::ResolveFlags::BENEATH
                            | rustix::fs::ResolveFlags::NO_SYMLINKS
                            | rustix::fs::ResolveFlags::NO_MAGICLINKS,
                    ),
                )
                .map(std::fs::File::from)
                .map_err(errno_text)?;
            if !file.metadata().map_err(|error| io_text(&error))?.is_file() {
                return Err(crate::i18n::tr("The file type changed during compression"));
            }
            Ok(ArchiveSource::File(file))
        }
        _ => Err(crate::i18n::tr(
            "Compression supports only regular files, folders, and symbolic links",
        )),
    }
}

/// Walks each path in `entries` and its descendants, calling `visit` per member.
///
/// Selected roots are opened from their parent directory via
/// [`open_local_parent_directory`], then recursion stays descriptor-relative
/// through [`visit_archive_entry`].
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set before or during the walk
/// - [`Failed`] if a root has no file name or parent, cannot be opened, or
///   `visit` fails
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn visit_archive_entries(
    entries: &[PathBuf],
    cancelled: &AtomicBool,
    visit: &mut impl FnMut(&Path, &ArchiveSource) -> Result<(), ArchiveError>,
) -> Result<(), ArchiveError> {
    for entry in entries {
        check_archive_cancelled(cancelled)?;
        let name = entry
            .file_name()
            .ok_or_else(|| crate::i18n::tr("Cannot compress the filesystem root"))?;
        let parent = open_local_parent_directory(
            entry
                .parent()
                .ok_or_else(|| crate::i18n::tr("Cannot compress the filesystem root"))?,
        )?;
        visit_archive_entry(&parent, name, Path::new(name), cancelled, visit)?;
    }
    Ok(())
}

/// Visits `name` inside `parent` and, for directories, each child beneath it.
///
/// `archive_path` is the member path written into the archive, rooted at the
/// originally selected entry's file name.
///
/// # Arguments
///
/// * `parent` - Already-open parent directory of `name`
/// * `name` - Directory entry to open without following symbolic links
/// * `archive_path` - Relative path recorded for this member
/// * `cancelled` - Flag checked before opening and before each child
/// * `visit` - Callback invoked with the archive path and opened source
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set
/// - [`Failed`] if the entry cannot be opened or `visit` fails
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn visit_archive_entry<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
    archive_path: &Path,
    cancelled: &AtomicBool,
    visit: &mut impl FnMut(&Path, &ArchiveSource) -> Result<(), ArchiveError>,
) -> Result<(), ArchiveError> {
    check_archive_cancelled(cancelled)?;
    let source = open_archive_source(parent, name).map_err(|error| {
        archive_failed(rust_i18n::t!(
            "Could not compress “%{path}”: %{error}",
            path = archive_path.display(),
            error = crate::services::error_detail(error)
        ))
    })?;
    visit(archive_path, &source)?;
    if let ArchiveSource::Directory(directory) = source {
        for child in local_directory_children(&directory)? {
            visit_archive_entry(
                &directory,
                &child,
                &archive_path.join(&child),
                cancelled,
                visit,
            )?;
        }
    }
    Ok(())
}

struct CompressionIo<'a, T> {
    inner: T,
    cancelled: &'a AtomicBool,
}

impl<'a, T> CompressionIo<'a, T> {
    fn new(inner: T, cancelled: &'a AtomicBool) -> Self {
        Self { inner, cancelled }
    }

    fn check(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(io::Error::other("Archive operation cancelled"))
        } else {
            Ok(())
        }
    }
}

impl<T: Read> Read for CompressionIo<'_, T> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        self.inner.read(buffer)
    }
}

impl<T: Write> Write for CompressionIo<'_, T> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.check()?;
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check()?;
        self.inner.flush()
    }
}

impl<T: Seek> Seek for CompressionIo<'_, T> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.check()?;
        self.inner.seek(position)
    }
}

fn compression_result(
    cancelled: &AtomicBool,
    encode: impl FnOnce() -> Result<(), ArchiveError>,
) -> Result<(), ArchiveError> {
    check_archive_cancelled(cancelled)?;
    let result = encode();
    // Encoders wrap I/O errors in their own types; cancellation remains a
    // terminal cancellation event rather than a password/error retry.
    check_archive_cancelled(cancelled)?;
    result
}

/// Writes a ZIP archive of `entries` into `file`.
///
/// Regular files use deflate level 6 unless [`is_incompressible`] selects
/// stored. An optional `password` enables AES-256 encryption. Symbolic-link
/// targets must be UTF-8; otherwise the caller is asked to use TAR instead.
///
/// # Arguments
///
/// * `file` - Staging file that receives the ZIP bytes
/// * `entries` - Absolute paths of selected files, directories, and links
/// * `password` - Optional AES-256 password
/// * `progress` - Counter incremented once per non-directory member
/// * `cancelled` - Flag checked between members and during copies
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set during the walk or copy
/// - [`Failed`] if a member cannot be opened, a symlink target is not UTF-8,
///   or ZIP encoding fails
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
pub(super) fn compress_zip(
    file: impl Write + Seek,
    entries: &[std::path::PathBuf],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    compression_result(cancelled, || {
        let writer =
            std::io::BufWriter::with_capacity(COPY_BUF, CompressionIo::new(file, cancelled));
        let mut writer = zip::ZipWriter::new(writer);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = if let Some(pw) = password {
            deflated.with_aes_encryption(zip::AesMode::Aes256, pw)
        } else {
            deflated
        };
        let stored = if let Some(pw) = password {
            stored.with_aes_encryption(zip::AesMode::Aes256, pw)
        } else {
            stored
        };
        visit_archive_entries(entries, cancelled, &mut |path, source| {
            let name = path.to_str().ok_or_else(|| {
                rust_i18n::t!(
                    "ZIP cannot preserve the non-UTF-8 name of %{path}. Use TAR instead.",
                    path = path.display()
                )
                .into_owned()
            })?;
            let (modified, mode) = source_metadata(source)?;
            match source {
                ArchiveSource::Directory(_) => {
                    return writer
                        .add_directory(name, zip_member_options(stored, modified, mode)?)
                        .map_err(|error| archive_failed(zip_text(error)));
                }
                ArchiveSource::Symlink { target, .. } => {
                    let target = target.to_str().ok_or_else(|| {
                        rust_i18n::t!(
                            "ZIP cannot preserve the non-UTF-8 link target of %{path}. Use TAR instead.",
                            path = path.display()
                        )
                        .into_owned()
                    })?;
                    writer
                        .add_symlink(name, target, zip_member_options(stored, modified, None)?)
                        .map_err(zip_text)?;
                }
                ArchiveSource::File(file) => {
                    let base = if is_incompressible(path) {
                        stored
                    } else {
                        deflated
                    };
                    writer
                        .start_file(name, zip_member_options(base, modified, mode)?)
                        .map_err(zip_text)?;
                    copy_with_big_buf(
                        std::io::BufReader::with_capacity(COPY_BUF, file),
                        &mut writer,
                        cancelled,
                    )?;
                }
            }
            progress.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })?;
        check_archive_cancelled(cancelled)?;
        writer
            .finish()
            .map_err(zip_text)?
            .into_inner()
            .map_err(|error| io_text(error.error()))?;
        Ok(())
    })
}

/// Writes a TAR archive of `entries` into `file`.
///
/// When `gzip` is set, the TAR stream is wrapped in one gzip member at that level.
/// Symbolic links are preserved as links.
///
/// # Arguments
///
/// * `file` - Staging file that receives the archive bytes
/// * `entries` - Absolute paths of selected files, directories, and links
/// * `gzip` - Stream-wide gzip level, or `None` for uncompressed TAR
/// * `progress` - Counter incremented once per non-directory member
/// * `cancelled` - Flag checked between members and during copies
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set during the walk or copy
/// - [`Failed`] if a member cannot be opened or TAR/gzip encoding fails
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
pub(super) fn compress_tar(
    file: impl Write,
    entries: &[std::path::PathBuf],
    gzip: Option<flate2::Compression>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    compression_result(cancelled, || {
        let writer =
            std::io::BufWriter::with_capacity(COPY_BUF, CompressionIo::new(file, cancelled));
        if let Some(level) = gzip {
            let mut encoder = flate2::write::GzEncoder::new(writer, level);
            append_tar_entries(&mut encoder, entries, progress, cancelled)?;
            encoder
                .finish()
                .map_err(|error| io_text(&error))?
                .into_inner()
                .map_err(|error| io_text(error.error()))?;
        } else {
            let mut writer = writer;
            append_tar_entries(&mut writer, entries, progress, cancelled)?;
            writer
                .into_inner()
                .map_err(|error| io_text(error.error()))?;
        }
        Ok(())
    })
}

/// Appends `entries` to an already-constructed TAR builder on `writer`.
///
/// # Arguments
///
/// * `writer` - Destination of the TAR stream, possibly a gzip encoder
/// * `entries` - Absolute paths of selected files, directories, and links
/// * `progress` - Counter incremented once per non-directory member
/// * `cancelled` - Flag checked between members
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set during the walk
/// - [`Failed`] if a member cannot be opened or TAR encoding fails
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn append_tar_entries(
    writer: &mut dyn std::io::Write,
    entries: &[std::path::PathBuf],
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    let mut builder = tar::Builder::new(CompressionIo::new(writer, cancelled));
    // The crate takes a file that reports no blocks as all holes, as small
    // files on ntfs3 or with ext4 inline data do, and would store zeros.
    builder.sparse(false);
    visit_archive_entries(entries, cancelled, &mut |path, source| {
        let mut header = tar::Header::new_gnu();
        match source {
            ArchiveSource::Symlink { target, modified } => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                header.set_uid(0);
                header.set_gid(0);
                if let Some(seconds) = modified.and_then(unix_seconds) {
                    header.set_mtime(seconds);
                }
                builder
                    .append_link(&mut header, path, target)
                    .map_err(|error| io_text(&error))?;
            }
            ArchiveSource::Directory(directory) => {
                header.set_metadata(&directory.metadata().map_err(|error| io_text(&error))?);
                return builder
                    .append_data(&mut header, path, std::io::empty())
                    .map_err(|error| archive_failed(io_text(&error)));
            }
            ArchiveSource::File(file) => {
                let mut file = file.try_clone().map_err(|error| io_text(&error))?;
                builder
                    .append_file(path, &mut file)
                    .map_err(|error| io_text(&error))?;
            }
        }
        check_archive_cancelled(cancelled)?;
        progress.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })?;
    check_archive_cancelled(cancelled)?;
    builder
        .finish()
        .map_err(|error| archive_failed(io_text(&error)))?;
    Ok(())
}

pub(super) struct ArchiveSources {
    pub files: usize,
    has_compressible_files: bool,
}

impl ArchiveSources {
    pub fn gzip_level(&self) -> flate2::Compression {
        if self.has_compressible_files {
            flate2::Compression::default()
        } else {
            // Gzip wraps the entire TAR stream: keep a valid gzip member, using
            // stored DEFLATE blocks only when every payload is already compressed.
            flate2::Compression::none()
        }
    }
}

/// Counts non-directory members and selects a stream-wide gzip policy without
/// reading payloads. Unknown formats retain compression; links are never followed.
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set during the walk
/// - [`Failed`] if a member cannot be opened
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
pub(super) fn inspect_archive_sources(
    entries: &[PathBuf],
    cancelled: &AtomicBool,
) -> Result<ArchiveSources, ArchiveError> {
    let mut sources = ArchiveSources {
        files: 0,
        has_compressible_files: false,
    };
    visit_archive_entries(entries, cancelled, &mut |path, source| {
        if !matches!(source, ArchiveSource::Directory(_)) {
            sources.files += 1;
        }
        if matches!(source, ArchiveSource::File(_)) && !is_incompressible(path) {
            sources.has_compressible_files = true;
        }
        Ok(())
    })?;
    Ok(sources)
}

/// A filename heuristic, not a claim about every possible payload in a container.
/// Unknown extensions and commonly raw containers (TAR, BMP, WAV, AVI, ISO) retain
/// compression. ZIP/7Z choose per member; gzip can only choose for the whole TAR.
const INCOMPRESSIBLE_EXTS: &[&str] = &[
    "zip", "7z", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "tzst", "rar", "lz", "lz4",
    "br", "mp4", "mkv", "mov", "webm", "flv", "wmv", "jpg", "jpeg", "png", "webp", "gif", "heic",
    "avif", "mp3", "flac", "aac", "ogg", "opus", "wma", "m4a", "pdf", "epub", "docx", "xlsx",
    "pptx", "odt", "ods", "odp", "deb", "rpm", "apk", "jar", "war",
];

/// Returns whether `path`'s extension is in [`INCOMPRESSIBLE_EXTS`].
fn is_incompressible(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| INCOMPRESSIBLE_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Writes a 7z archive of `entries` into `file`.
///
/// Uses LZMA2 at level 6 with a thread count from
/// [`std::thread::available_parallelism`]. An optional `password` adds AES
/// encryption. Symbolic links are not supported.
///
/// # Arguments
///
/// * `file` - Staging file that receives the 7z bytes
/// * `entries` - Absolute paths of selected files and directories
/// * `password` - Optional AES password
/// * `progress` - Counter incremented once per non-directory member
/// * `cancelled` - Flag checked between members
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set during the walk
/// - [`Failed`] if a member cannot be opened, a symbolic link is selected,
///   or 7z encoding fails
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
pub(super) fn compress_7z(
    file: impl Write + Seek,
    entries: &[std::path::PathBuf],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, EncoderOptions, Lzma2Options};
    compression_result(cancelled, || {
        let mut writer = sevenz_rust2::ArchiveWriter::new(CompressionIo::new(file, cancelled))
            .map_err(sevenz_text)?;
        let lzma2 = sevenz_rust2::EncoderConfiguration::new(sevenz_rust2::EncoderMethod::LZMA2)
            .with_options(EncoderOptions::Lzma2(Lzma2Options::from_level(6)));
        let copy = sevenz_rust2::EncoderConfiguration::new(sevenz_rust2::EncoderMethod::COPY);
        let methods = |codec| {
            if let Some(pw) = password {
                // The last coder sees plaintext. Keep AES first for both codecs,
                // including the final configuration used to encrypt the header.
                vec![AesEncoderOptions::new(pw.into()).into(), codec]
            } else {
                vec![codec]
            }
        };
        let compressed_methods = methods(lzma2);
        let stored_methods = methods(copy);
        writer.set_content_methods(compressed_methods.clone());
        visit_archive_entries(entries, cancelled, &mut |path, source| {
            let name = path.to_str().ok_or_else(|| {
                archive_failed(rust_i18n::t!(
                    "7z cannot preserve the non-UTF-8 name of %{path}. Use TAR instead.",
                    path = path.display()
                ))
            })?;
            let (mut entry, file) = match source {
                ArchiveSource::Symlink { .. } => {
                    return Err(archive_failed(rust_i18n::t!(
                        "7z compression does not support symbolic links: %{path}. Use ZIP or TAR instead.",
                        path = path.display()
                    )));
                }
                ArchiveSource::Directory(file) => {
                    (sevenz_rust2::ArchiveEntry::new_directory(name), file)
                }
                ArchiveSource::File(file) => (sevenz_rust2::ArchiveEntry::new_file(name), file),
            };
            let metadata = file.metadata().map_err(|error| io_text(&error))?;
            entry.has_windows_attributes = true;
            entry.windows_attributes = FILE_ATTRIBUTE_UNIX_EXTENSION
                | (metadata.permissions().mode() << 16)
                | if metadata.is_dir() { 0x10 } else { 0x20 };
            if is_incompressible(path) {
                writer.set_content_methods(stored_methods.clone());
            } else {
                // Members are encoded independently. Avoid a 64 MiB work queue and
                // detached codec threads; cap the dictionary to the member size.
                let mut options = Lzma2Options::from_level(6);
                options.set_dictionary_size(metadata.len().min(8 << 20) as u32);
                let codec =
                    sevenz_rust2::EncoderConfiguration::new(sevenz_rust2::EncoderMethod::LZMA2)
                        .with_options(EncoderOptions::Lzma2(options));
                writer.set_content_methods(methods(codec));
            }
            if let Ok(modified) = metadata.modified()
                && let Ok(date) = sevenz_rust2::NtTime::try_from(modified)
            {
                entry.last_modified_date = date;
                entry.has_last_modified_date = u64::from(date) > 0;
            }
            if let Ok(created) = metadata.created()
                && let Ok(date) = sevenz_rust2::NtTime::try_from(created)
            {
                entry.creation_date = date;
                entry.has_creation_date = u64::from(date) > 0;
            }
            if let Ok(accessed) = metadata.accessed()
                && let Ok(date) = sevenz_rust2::NtTime::try_from(accessed)
            {
                entry.access_date = date;
                entry.has_access_date = u64::from(date) > 0;
            }
            let reader = if matches!(source, ArchiveSource::Directory(_)) {
                None
            } else {
                Some(CompressionIo::new(file, cancelled))
            };
            let has_reader = reader.is_some();
            writer
                .push_archive_entry(entry, reader)
                .map_err(|error| archive_failed(sevenz_text(error)))?;
            check_archive_cancelled(cancelled)?;
            if has_reader {
                progress.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        })?;
        check_archive_cancelled(cancelled)?;
        writer
            .finish()
            .map_err(|error| archive_failed(io_text(&error)))?;
        Ok(())
    })
}
