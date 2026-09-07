// SPDX-License-Identifier: GPL-3.0-or-later

//! Local archive compression and extraction for [`LocalOperationProvider`].
//!
//! Builds and unpacks ZIP, 7z, TAR, and gzip-compressed TAR archives on the
//! local filesystem. Compression writes through a `0o600` staging file and
//! publishes the result only after the encoder finishes, so a partial archive
//! is never left at the destination name. Extraction pins the destination
//! directory and creates members with `openat`/`mkdirat` and `NOFOLLOW`, after
//! [`validated_archive_path`] rejects absolute paths, `..`, and Windows drive
//! prefixes.
//!
//! # Main entry points
//!
//! - [`compress`] — start a cancellable compression
//! - [`extract`] — start a cancellable extraction
//!
//! Both spawn work on the default [`glib::MainContext`] and return a
//! [`LoadHandle`] that sets a cancellation flag when dropped.
//!
//! [`LocalOperationProvider`]: super::LocalOperationProvider

#[cfg(test)]
mod tests;

use std::{
    cell::RefCell,
    collections::HashSet,
    ffi::{OsStr, OsString},
    io,
    os::{
        fd::{AsFd, OwnedFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::PermissionsExt,
        },
    },
    path::{Component, Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use gtk::{gio, glib};

use super::{local_directory_children, open_local_child_directory, open_local_parent_directory};
use crate::{
    model::Location,
    services::{
        ArchiveFormat, CancelledOperation, CompressRequest, ExtractRequest, LoadHandle,
        OperationEvent, OperationRequestId, TransferConflict, validate_basename,
    },
};

/// Writes an archive through a staging file, then publishes it at `archive_path`.
///
/// Creates a `.strata-compression-` tempfile in `destination` with mode `0o600`,
/// runs `write_archive` on a worker thread, applies the published permissions,
/// and persists the file according to `conflict`. [`FailIfExists`] refuses to
/// replace an existing archive; [`ReplaceExisting`] overwrites it and copies
/// the current destination file's mode when that path is already a regular
/// file. Otherwise the published mode is `0o666` masked by the process umask.
///
/// # Arguments
///
/// * `destination` - Directory that holds both the staging file and the final archive
/// * `archive_path` - Final path to publish once encoding succeeds
/// * `conflict` - Whether an existing archive may be replaced
/// * `cancelled` - Flag checked after encoding and before persist
/// * `write_archive` - Encoder that writes the archive body into the staging file
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set after encoding finishes
/// - [`Failed`] if staging, encoding, permission updates, or persist fail, including
///   when the compression task panics
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
/// [`FailIfExists`]: TransferConflict::FailIfExists
/// [`ReplaceExisting`]: TransferConflict::ReplaceExisting
async fn write_staged_archive<F>(
    destination: &Path,
    archive_path: &Path,
    conflict: TransferConflict,
    cancelled: &AtomicBool,
    write_archive: F,
) -> Result<(), ArchiveError>
where
    F: FnOnce(std::fs::File) -> Result<(), ArchiveError> + Send + 'static,
{
    let published_permissions = if conflict == TransferConflict::ReplaceExisting {
        match std::fs::symlink_metadata(archive_path) {
            Ok(metadata) if metadata.file_type().is_file() => Some(metadata.permissions()),
            Ok(_) => None,
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(archive_failed(error)),
        }
    } else {
        None
    }
    .unwrap_or_else(umask_adjusted_file_permissions);
    let mut builder = tempfile::Builder::new();
    builder
        .prefix(".strata-compression-")
        .permissions(std::fs::Permissions::from_mode(0o600));
    let staged = builder.tempfile_in(destination).map_err(archive_failed)?;
    let file = staged.reopen().map_err(archive_failed)?;
    gio::spawn_blocking(move || write_archive(file))
        .await
        .map_err(|_| archive_failed("Compression task panicked"))??;
    check_archive_cancelled(cancelled)?;
    staged
        .as_file()
        .set_permissions(published_permissions)
        .map_err(archive_failed)?;
    match conflict {
        TransferConflict::FailIfExists => staged.persist_noclobber(archive_path),
        TransferConflict::ReplaceExisting => staged.persist(archive_path),
    }
    .map(|_| ())
    .map_err(archive_failed)
}

/// Returns `0o666` masked by the process umask from [`process_umask`].
fn umask_adjusted_file_permissions() -> std::fs::Permissions {
    std::fs::Permissions::from_mode(0o666 & !process_umask())
}

/// Reads the process umask from `/proc/self/status`.
///
/// Avoids the process-global `umask(2)` set-and-restore race that would
/// otherwise be unsafe in a multi-threaded GUI. Returns `0o022` when `/proc`
/// is unavailable or the `Umask:` line cannot be parsed.
fn process_umask() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("Umask:")
                    .and_then(|value| u32::from_str_radix(value.trim(), 8).ok())
            })
        })
        .unwrap_or(0o022)
}

/// Compresses the entries in `request` into a new local archive.
///
/// Validates that the destination is a local path and that
/// [`CompressRequest::archive_name`] passes [`validate_basename`], then writes
/// `{name}.{extension}` through [`write_staged_archive`]. Progress is polled
/// every 100 ms via [`archive_progress_timer`]. Returns a [`LoadHandle`] that
/// cancels in-flight work when dropped.
///
/// Emits [`ArchiveStarted`] immediately, [`ArchiveProgress`] while running,
/// then [`Compressed`], [`Failed`], or [`Cancelled`] depending on the outcome.
///
/// # Concurrency
///
/// Runs on the default [`glib::MainContext`]. Archive encoding happens on a
/// worker thread via [`gio::spawn_blocking`].
///
/// [`ArchiveStarted`]: OperationEvent::ArchiveStarted
/// [`ArchiveProgress`]: OperationEvent::ArchiveProgress
/// [`Compressed`]: OperationEvent::Compressed
/// [`Failed`]: OperationEvent::Failed
/// [`Cancelled`]: OperationEvent::Cancelled
pub(super) fn compress(request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = cancelled.clone();
    let work_cancelled = cancelled.clone();
    let destination = request.destination.clone();
    let source_locations = request
        .entries
        .iter()
        .map(|entry| entry.location.clone())
        .collect::<Vec<_>>();
    let _task = glib::MainContext::default().spawn_local(async move {
        let Some(dest_dir) = request.destination.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Archive destination must be a local path".to_owned(),
            });
            return;
        };
        if let Err(message) = validate_basename(&request.archive_name) {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: message.to_owned(),
            });
            return;
        }
        let archive_name = format!("{}.{}", request.archive_name, request.format.extension());
        let archive_path = dest_dir.join(&archive_name);
        let entries: Vec<std::path::PathBuf> = request
            .entries
            .iter()
            .filter_map(|e| e.location.native_path().map(Path::to_path_buf))
            .collect();
        if entries.is_empty() {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Nothing to compress".to_owned(),
            });
            return;
        }
        let total = Arc::new(AtomicUsize::new(0));
        let progress = Arc::new(AtomicUsize::new(0));
        emit(OperationEvent::ArchiveStarted {
            request_id: request.id,
            total: 0,
        });
        let timer_id =
            archive_progress_timer(request.id, &progress, &total, &task_cancelled, &emit);
        let format = request.format;
        let password = request.password.clone();
        let work_progress = progress.clone();
        let work_total = total.clone();
        let result = write_staged_archive(
            &dest_dir,
            &archive_path,
            request.conflict,
            &task_cancelled,
            move |file| {
                let count = count_archive_files(&entries, &work_cancelled)?;
                work_total.store(count, Ordering::Relaxed);
                match format {
                    ArchiveFormat::Zip => compress_zip(
                        file,
                        &entries,
                        password.as_deref(),
                        &work_progress,
                        &work_cancelled,
                    ),
                    ArchiveFormat::SevenZ => compress_7z(
                        file,
                        &entries,
                        password.as_deref(),
                        &work_progress,
                        &work_cancelled,
                    ),
                    ArchiveFormat::TarGz => {
                        compress_tar(file, &entries, true, &work_progress, &work_cancelled)
                    }
                    ArchiveFormat::Tar => {
                        compress_tar(file, &entries, false, &work_progress, &work_cancelled)
                    }
                }
            },
        )
        .await;
        timer_id.remove();
        match result {
            Ok(()) => emit(OperationEvent::Compressed {
                request_id: request.id,
                archive_name: archive_name.clone(),
            }),
            Err(ArchiveError::Cancelled) => emit(cancelled_archive_event(
                request.id,
                destination,
                Vec::new(),
                Vec::new(),
                source_locations,
            )),
            Err(ArchiveError::Failed(error)) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: error,
            }),
        }
    });
    LoadHandle::new(move || {
        cancelled.store(true, Ordering::Relaxed);
    })
}

/// Extracts the archive in `request` into a local destination directory.
///
/// Requires both the archive and destination to be local paths. Format is
/// inferred from [`FileEntry::display_name`] via [`ArchiveFormat::from_extension`].
/// Returns a [`LoadHandle`] that cancels in-flight work when dropped.
///
/// Emits [`ArchiveStarted`] immediately, [`ArchiveProgress`] while running,
/// then [`Extracted`], [`Failed`], or [`Cancelled`]. A cancel after some
/// members have been written reports completed, failed, and not-attempted
/// locations through [`CancelledOperation`].
///
/// # Concurrency
///
/// Runs on the default [`glib::MainContext`]. Decoding happens on a worker
/// thread via [`gio::spawn_blocking`].
///
/// [`FileEntry::display_name`]: crate::model::FileEntry::display_name
/// [`ArchiveStarted`]: OperationEvent::ArchiveStarted
/// [`ArchiveProgress`]: OperationEvent::ArchiveProgress
/// [`Extracted`]: OperationEvent::Extracted
/// [`Failed`]: OperationEvent::Failed
/// [`Cancelled`]: OperationEvent::Cancelled
pub(super) fn extract(request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = cancelled.clone();
    let work_cancelled = cancelled.clone();
    let destination = request.destination.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let Some(archive_path) = request.entry.location.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Archive must be a local file".to_owned(),
            });
            return;
        };
        let Some(dest_dir) = request.destination.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Extract destination must be a local path".to_owned(),
            });
            return;
        };
        let format = ArchiveFormat::from_extension(&request.entry.display_name);
        let password = request.password.clone();
        let display_name = request.entry.display_name.clone();
        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        emit(OperationEvent::ArchiveStarted {
            request_id: request.id,
            total: 0,
        });
        let timer_id =
            archive_progress_timer(request.id, &progress, &total, &task_cancelled, &emit);
        let work_progress = progress.clone();
        let work_total = total.clone();
        let result = gio::spawn_blocking(move || match format {
            Some(ArchiveFormat::Zip) => {
                let file = std::fs::File::open(&archive_path).map_err(|e| e.to_string())?;
                let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                work_total.store(archive.len(), Ordering::Relaxed);
                extract_zip_from_archive(
                    &mut archive,
                    &dest_dir,
                    password.as_deref(),
                    &work_progress,
                    &work_cancelled,
                )
            }
            Some(ArchiveFormat::SevenZ) => {
                let pw = password
                    .as_deref()
                    .map(sevenz_rust2::Password::from)
                    .unwrap_or_default();
                let file = std::fs::File::open(&archive_path).map_err(|e| e.to_string())?;
                extract_7z_from_reader(file, &dest_dir, pw, &work_progress, &work_cancelled)
            }
            Some(ArchiveFormat::TarGz) => extract_tar(
                &archive_path,
                &dest_dir,
                true,
                &work_progress,
                &work_cancelled,
            ),
            Some(ArchiveFormat::Tar) => extract_tar(
                &archive_path,
                &dest_dir,
                false,
                &work_progress,
                &work_cancelled,
            ),
            None => Err(archive_failed(format!(
                "Unsupported archive format: {display_name}"
            ))),
        })
        .await;
        timer_id.remove();
        match result {
            Ok(Ok(ArchiveOutcome::Completed(first_name))) => emit(OperationEvent::Extracted {
                request_id: request.id,
                first_name,
            }),
            Ok(Ok(ArchiveOutcome::Cancelled {
                completed,
                failed,
                not_attempted,
            })) => emit(cancelled_archive_event(
                request.id,
                destination,
                completed,
                failed,
                not_attempted,
            )),
            Ok(Err(ArchiveError::Cancelled)) => emit(cancelled_archive_event(
                request.id,
                destination,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )),
            Ok(Err(ArchiveError::Failed(error))) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: error,
            }),
            Err(_) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Extraction task panicked".to_owned(),
            }),
        }
    });
    LoadHandle::new(move || {
        cancelled.store(true, Ordering::Relaxed);
    })
}

/// An opened compression source, re-read from disk relative to its parent
/// directory rather than trusted from any earlier listing.
enum ArchiveSource {
    /// Open file description for a regular file, opened with `BENEATH`,
    /// `NO_SYMLINKS`, and `NO_MAGICLINKS`.
    File(std::fs::File),
    /// Open directory used to walk children descriptor-relative.
    Directory(std::fs::File),
    /// Symlink target as stored, archived as a link rather than followed.
    Symlink(PathBuf),
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
    let stat = rustix::fs::statat(parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| error.to_string())?;
    match rustix::fs::FileType::from_raw_mode(stat.st_mode) {
        rustix::fs::FileType::Symlink => {
            let target = rustix::fs::readlinkat(parent, name, Vec::new())
                .map_err(|error| error.to_string())?;
            Ok(ArchiveSource::Symlink(PathBuf::from(OsString::from_vec(
                target.into_bytes(),
            ))))
        }
        rustix::fs::FileType::Directory => open_local_child_directory(parent, name)
            .map(std::fs::File::from)
            .map(ArchiveSource::Directory),
        rustix::fs::FileType::RegularFile => {
            let file = rustix::fs::openat2(
                parent,
                name,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::NONBLOCK
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
                rustix::fs::ResolveFlags::BENEATH
                    | rustix::fs::ResolveFlags::NO_SYMLINKS
                    | rustix::fs::ResolveFlags::NO_MAGICLINKS,
            )
            .map(std::fs::File::from)
            .map_err(|error| error.to_string())?;
            if !file
                .metadata()
                .map_err(|error| error.to_string())?
                .is_file()
            {
                return Err("The file type changed during compression".to_owned());
            }
            Ok(ArchiveSource::File(file))
        }
        _ => Err("Compression supports only regular files, folders, and symbolic links".to_owned()),
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
        let name = entry.file_name().ok_or("Entry has no file name")?;
        let parent = open_local_parent_directory(entry.parent().ok_or("Entry has no parent")?)?;
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
        archive_failed(format!(
            "Could not compress {}: {error}",
            archive_path.display()
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
fn compress_zip(
    file: std::fs::File,
    entries: &[std::path::PathBuf],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    let writer = std::io::BufWriter::with_capacity(COPY_BUF, file);
    let mut writer = zip::ZipWriter::new(writer);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
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
        let name = path.to_string_lossy();
        match source {
            ArchiveSource::Directory(_) => {
                return writer.add_directory(name, stored).map_err(archive_failed);
            }
            ArchiveSource::Symlink(target) => {
                let target = target.to_str().ok_or_else(|| {
                    format!(
                        "ZIP cannot preserve the non-UTF-8 link target of {}. Use TAR instead.",
                        path.display()
                    )
                })?;
                writer
                    .add_symlink(name, target, stored)
                    .map_err(|error| error.to_string())?;
            }
            ArchiveSource::File(file) => {
                let options = if is_incompressible(path) {
                    stored
                } else {
                    deflated
                };
                writer
                    .start_file(name, options)
                    .map_err(|error| error.to_string())?;
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
        .map_err(|error| error.to_string())?
        .into_inner()
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Writes a TAR archive of `entries` into `file`.
///
/// When `gzip` is true, the TAR stream is wrapped in a gzip encoder.
/// Symbolic links are preserved as links.
///
/// # Arguments
///
/// * `file` - Staging file that receives the archive bytes
/// * `entries` - Absolute paths of selected files, directories, and links
/// * `gzip` - Whether to wrap the TAR stream in gzip
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
fn compress_tar(
    file: std::fs::File,
    entries: &[std::path::PathBuf],
    gzip: bool,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    let writer = std::io::BufWriter::with_capacity(COPY_BUF, file);
    if gzip {
        let mut encoder = flate2::write::GzEncoder::new(writer, flate2::Compression::default());
        append_tar_entries(&mut encoder, entries, progress, cancelled)?;
        encoder
            .finish()
            .map_err(|error| error.to_string())?
            .into_inner()
            .map_err(|error| error.to_string())?;
    } else {
        let mut writer = writer;
        append_tar_entries(&mut writer, entries, progress, cancelled)?;
        writer.into_inner().map_err(|error| error.to_string())?;
    }
    Ok(())
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
    let mut builder = tar::Builder::new(writer);
    visit_archive_entries(entries, cancelled, &mut |path, source| {
        let mut header = tar::Header::new_gnu();
        match source {
            ArchiveSource::Symlink(target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                header.set_uid(0);
                header.set_gid(0);
                builder
                    .append_link(&mut header, path, target)
                    .map_err(|error| error.to_string())?;
            }
            ArchiveSource::Directory(directory) => {
                header.set_metadata(&directory.metadata().map_err(|error| error.to_string())?);
                return builder
                    .append_data(&mut header, path, std::io::empty())
                    .map_err(archive_failed);
            }
            ArchiveSource::File(file) => {
                let mut file = file.try_clone().map_err(|error| error.to_string())?;
                builder
                    .append_file(path, &mut file)
                    .map_err(|error| error.to_string())?;
            }
        }
        check_archive_cancelled(cancelled)?;
        progress.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })?;
    check_archive_cancelled(cancelled)?;
    builder.finish().map_err(archive_failed)?;
    Ok(())
}

/// Converts an archive member name into a relative path that cannot escape the destination.
///
/// Normalizes backslashes to slashes, skips empty and `.` components, and
/// rejects absolute paths, `..`, Windows drive prefixes (`C:`), and names
/// that collapse to empty.
///
/// # Errors
///
/// Returns an error if `name` is empty, absolute, contains `..`, includes a
/// drive prefix, or has no remaining components after normalization.
fn validated_archive_path(name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') {
        return Err(format!("Refusing unsafe archive path: {name}"));
    }

    let mut path = PathBuf::new();
    for component in normalized.split('/') {
        match component.as_bytes() {
            b"" | b"." => {}
            b".." => return Err(format!("Refusing unsafe archive path: {name}")),
            bytes if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' => {
                return Err(format!("Refusing unsafe archive path: {name}"));
            }
            _ => path.push(component),
        }
    }
    if path.as_os_str().is_empty() {
        return Err(format!("Refusing empty archive path: {name}"));
    }
    Ok(path)
}

/// Returns `name` with ` ({index})` inserted before the extension.
///
/// Used by [`ExtractionDestination::available_name`] to pick `readme (2).txt`
/// when `readme.txt` already exists.
fn suffixed_name(name: &OsStr, index: u64) -> OsString {
    let path = Path::new(name);
    let mut candidate = path.file_stem().unwrap_or(name).as_bytes().to_vec();
    candidate.extend_from_slice(format!(" ({index})").as_bytes());
    if let Some(extension) = path.extension() {
        candidate.push(b'.');
        candidate.extend_from_slice(extension.as_bytes());
    }
    OsString::from_vec(candidate)
}

/// Pinned destination directory for extraction.
///
/// All member creates go through this root with `NOFOLLOW`, so a symlink
/// swapped into the destination tree cannot redirect writes outside it.
struct ExtractionDestination {
    root: OwnedFd,
}

impl ExtractionDestination {
    /// Opens `path` as a directory without following a final symbolic link.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` cannot be opened as a directory.
    fn open(path: &Path) -> Result<Self, String> {
        let root = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| format!("Could not open extraction destination: {error}"))?;
        Ok(Self { root })
    }

    /// Finds a name in `directory` that does not already exist.
    ///
    /// Tries `name`, then [`suffixed_name`] with increasing indexes. Existing
    /// regular files and directories are skipped; special filesystem objects
    /// (devices, sockets, existing symlinks) are refused rather than overwritten.
    ///
    /// # Errors
    ///
    /// Returns an error if `directory` cannot be inspected or an existing
    /// candidate is a special filesystem object.
    fn available_name<Fd: AsFd>(&self, directory: &Fd, name: &OsStr) -> Result<OsString, String> {
        for index in 1.. {
            let candidate = if index == 1 {
                name.to_owned()
            } else {
                suffixed_name(name, index)
            };
            match rustix::fs::statat(directory, &candidate, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
                Err(rustix::io::Errno::NOENT) => return Ok(candidate),
                Err(error) => {
                    return Err(format!(
                        "Could not inspect extraction path {}: {error}",
                        candidate.to_string_lossy()
                    ));
                }
                Ok(stat) => match rustix::fs::FileType::from_raw_mode(stat.st_mode) {
                    rustix::fs::FileType::RegularFile | rustix::fs::FileType::Directory => {}
                    _ => {
                        return Err(format!(
                            "Refusing to extract over special filesystem object: {}",
                            candidate.to_string_lossy()
                        ));
                    }
                },
            }
        }
        Err(format!(
            "Could not find an available extraction name for {}",
            name.to_string_lossy()
        ))
    }

    /// Creates each component of `path` under the destination root and returns the leaf directory.
    ///
    /// Existing directories are reused. Each component is opened with
    /// [`DIRECTORY`] and [`NOFOLLOW`], so a symlink cannot be followed as a
    /// directory.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` contains a non-normal component, a component
    /// cannot be created, or a component exists but is not a directory.
    ///
    /// [`DIRECTORY`]: rustix::fs::OFlags::DIRECTORY
    /// [`NOFOLLOW`]: rustix::fs::OFlags::NOFOLLOW
    fn create_directories(&self, path: &Path) -> Result<OwnedFd, String> {
        let mut directory = self.root.try_clone().map_err(|error| error.to_string())?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                return Err("Invalid internal extraction path".to_owned());
            };
            match rustix::fs::mkdirat(&directory, name, rustix::fs::Mode::from_raw_mode(0o777)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(error) => return Err(error.to_string()),
            }
            directory = rustix::fs::openat(
                &directory,
                name,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(directory)
    }

    /// Creates the file at `path`, renaming the leaf if that name is already taken.
    ///
    /// Parent directories are created with [`Self::create_directories`]. The
    /// leaf is opened with [`CREATE`], [`EXCL`], and [`NOFOLLOW`] so an existing
    /// file or symlink is never overwritten. Returns the open file and the
    /// relative path actually created, which may differ from `path` after a
    /// rename.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` has no file name, a parent cannot be created,
    /// no unused name can be found, or the exclusive create fails.
    ///
    /// [`CREATE`]: rustix::fs::OFlags::CREATE
    /// [`EXCL`]: rustix::fs::OFlags::EXCL
    /// [`NOFOLLOW`]: rustix::fs::OFlags::NOFOLLOW
    fn create_file(&self, path: &Path) -> Result<(std::fs::File, PathBuf), String> {
        let parent = self.create_directories(path.parent().unwrap_or_else(|| Path::new("")))?;
        let name = path
            .file_name()
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        let name = self.available_name(&parent, name)?;
        let mut created = PathBuf::new();
        if let Some(parent_path) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            created.push(parent_path);
        }
        created.push(&name);
        let file = rustix::fs::openat(
            parent,
            name,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(0o666),
        )
        .map(std::fs::File::from)
        .map_err(|error| error.to_string())?;
        Ok((file, created))
    }

    /// Unlinks the leaf of `path` under the destination root.
    ///
    /// Used to discard a partially written member after cancellation or
    /// copy failure. Does not follow a final symbolic link.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` has no file name, a parent cannot be opened,
    /// or the unlink fails.
    fn remove_file(&self, path: &Path) -> Result<(), String> {
        let parent = self.create_directories(path.parent().unwrap_or_else(|| Path::new("")))?;
        let name = path
            .file_name()
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        rustix::fs::unlinkat(&parent, name, rustix::fs::AtFlags::empty()).map_err(|error| {
            format!(
                "Could not remove incomplete extraction {}: {error}",
                path.display()
            )
        })
    }
}

/// Counts non-directory members under `entries` for progress totals.
///
/// Directories are visited so their children are counted, but the directories
/// themselves are excluded from the total.
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set during the walk
/// - [`Failed`] if a member cannot be opened
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn count_archive_files(entries: &[PathBuf], cancelled: &AtomicBool) -> Result<usize, ArchiveError> {
    let mut count = 0;
    visit_archive_entries(entries, cancelled, &mut |_, source| {
        if !matches!(source, ArchiveSource::Directory(_)) {
            count += 1;
        }
        Ok(())
    })?;
    Ok(count)
}

/// Byte size of the reusable read/write buffer used by [`copy_with_big_buf`].
const COPY_BUF: usize = 1 << 20;
/// Sentinel message used to round-trip cancellation through `sevenz_rust2`.
const ARCHIVE_CANCELLED: &str = "Operation cancelled";

/// Failure or cooperative cancellation of a compress or extract step.
#[derive(Debug, PartialEq, Eq)]
enum ArchiveError {
    /// The [`LoadHandle`] cancelled the operation before it finished.
    Cancelled,
    /// Encoding, decoding, or filesystem work failed with this message.
    Failed(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str(ARCHIVE_CANCELLED),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<String> for ArchiveError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<&str> for ArchiveError {
    fn from(message: &str) -> Self {
        Self::Failed(message.to_owned())
    }
}

fn archive_failed(error: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::Failed(error.to_string())
}

/// Builds the `sevenz_rust2` error that [`sevenz_is_cancelled`] recognizes.
fn sevenz_cancelled() -> sevenz_rust2::Error {
    sevenz_rust2::Error::Other(ARCHIVE_CANCELLED.into())
}

/// Returns whether `error` is the cancellation sentinel from [`sevenz_cancelled`].
fn sevenz_is_cancelled(error: &sevenz_rust2::Error) -> bool {
    matches!(error, sevenz_rust2::Error::Other(message) if message.as_ref() == ARCHIVE_CANCELLED)
}

/// Result of an extract that may stop after writing some members.
enum ArchiveOutcome<T> {
    /// Every member was processed; `T` is typically the first created name.
    Completed(T),
    /// Work stopped early. Location lists feed [`CancelledOperation`].
    Cancelled {
        completed: Vec<Location>,
        failed: Vec<Location>,
        not_attempted: Vec<Location>,
    },
}

/// Returns [`ArchiveError::Cancelled`] when the `cancelled` flag is set.
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set
///
/// [`Cancelled`]: ArchiveError::Cancelled
fn check_archive_cancelled(cancelled: &AtomicBool) -> Result<(), ArchiveError> {
    if cancelled.load(Ordering::Relaxed) {
        Err(ArchiveError::Cancelled)
    } else {
        Ok(())
    }
}

/// Builds an [`OperationEvent::Cancelled`] for a compress or extract request.
///
/// # Arguments
///
/// * `request_id` - Identifier of the cancelled request
/// * `destination` - Archive path or extract directory, recorded as affected
/// * `completed` - Members fully written before cancellation
/// * `failed` - Members left in a partial or unremovable state
/// * `not_attempted` - Members not started, including a cleaned-up partial write
fn cancelled_archive_event(
    request_id: OperationRequestId,
    destination: Location,
    completed: Vec<Location>,
    failed: Vec<Location>,
    not_attempted: Vec<Location>,
) -> OperationEvent {
    OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed,
            failed,
            not_attempted,
            affected_locations: HashSet::from([destination]),
        },
    }
}

/// Builds a [`Location`] for `relative` under the extract `destination`.
fn extract_entry_location(destination: &Path, relative: &Path) -> Location {
    Location::local(destination.join(relative))
}

/// Collects extract destinations for ZIP members from index `from` onward.
fn zip_entry_locations(
    archive: &zip::ZipArchive<std::fs::File>,
    destination: &Path,
    from: usize,
) -> Vec<Location> {
    (from..archive.len())
        .filter_map(|index| archive.name_for_index(index))
        .map(|name| Location::local(destination.join(name)))
        .collect()
}

/// Maps a TAR entry to its extract destination, ignoring unsafe member names.
fn tar_entry_location<'a, R: std::io::Read + 'a>(
    entry: tar::Entry<'a, R>,
    dest_dir: &Path,
) -> Option<Location> {
    let name = entry.path().ok()?;
    let path = validated_archive_path(&name.to_string_lossy()).ok()?;
    Some(extract_entry_location(dest_dir, &path))
}

/// Collects extract destinations for 7z members at or after `from_name`.
///
/// When `skip_current` is true, `from_name` itself is omitted — used after a
/// partial write that was either cleaned up or recorded as failed.
///
/// # Arguments
///
/// * `dest_dir` - Extraction root used to build [`Location`] values
/// * `names` - Archive member names in archive order
/// * `from_name` - Member at which to start collecting
/// * `skip_current` - Whether to omit `from_name` from the result
fn sevenz_locations_from(
    dest_dir: &Path,
    names: &[String],
    from_name: &str,
    skip_current: bool,
) -> Vec<Location> {
    let Some(index) = names.iter().position(|name| name == from_name) else {
        return Vec::new();
    };
    let start = if skip_current { index + 1 } else { index };
    names
        .get(start..)
        .unwrap_or(&[])
        .iter()
        .filter_map(|name| validated_archive_path(name).ok())
        .map(|path| extract_entry_location(dest_dir, &path))
        .collect()
}

/// Classifies a cancelled extract after a member was partially written.
///
/// If `removed` succeeded, the interrupted path is treated as not-attempted
/// along with `remaining`. If cleanup failed, that path is recorded as failed
/// and `remaining` stays not-attempted.
///
/// # Arguments
///
/// * `dest_dir` - Extraction root used to build [`Location`] values
/// * `created` - Relative path of the interrupted member
/// * `completed` - Members fully written before the interruption
/// * `remaining` - Members not yet opened
/// * `removed` - Result of unlinking the partial file
fn cancelled_extract_after_partial_write(
    dest_dir: &Path,
    created: &Path,
    completed: Vec<Location>,
    remaining: Vec<Location>,
    removed: Result<(), String>,
) -> ArchiveOutcome<Option<String>> {
    let interrupted = extract_entry_location(dest_dir, created);
    if removed.is_ok() {
        let mut not_attempted = vec![interrupted];
        not_attempted.extend(remaining);
        ArchiveOutcome::Cancelled {
            completed,
            failed: Vec::new(),
            not_attempted,
        }
    } else {
        ArchiveOutcome::Cancelled {
            completed,
            failed: vec![interrupted],
            not_attempted: remaining,
        }
    }
}

/// Copies `reader` to `writer`, checking `cancelled` between 1 MiB chunks.
///
/// Returns the number of bytes written.
///
/// # Performance
///
/// Uses a [`COPY_BUF`]-sized heap buffer so large members are not copied
/// through the default 8 KiB [`std::io::copy`] path.
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set between reads
/// - [`Failed`] if a read or write fails
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn copy_with_big_buf(
    mut reader: impl std::io::Read,
    writer: &mut (impl std::io::Write + ?Sized),
    cancelled: &AtomicBool,
) -> Result<u64, ArchiveError> {
    let mut buf = vec![0u8; COPY_BUF];
    let mut total = 0;
    loop {
        check_archive_cancelled(cancelled)?;
        let n = reader.read(&mut buf).map_err(archive_failed)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).map_err(archive_failed)?;
        total += n as u64;
    }
    Ok(total)
}

/// Starts a 100 ms timer that emits [`OperationEvent::ArchiveProgress`].
///
/// The source stays attached until the caller removes the returned
/// [`glib::SourceId`]. Returning [`glib::ControlFlow::Break`] from the
/// callback would double-remove that source.
///
/// # Arguments
///
/// * `request_id` - Identifier included in each progress event
/// * `progress` - Completed member count polled each tick
/// * `total` - Expected member count, which may still be zero during counting
/// * `cancelled` - When set, ticks stop emitting but the source stays attached
/// * `emit` - Callback that receives progress events on the main context
fn archive_progress_timer(
    request_id: OperationRequestId,
    progress: &Arc<AtomicUsize>,
    total: &Arc<AtomicUsize>,
    cancelled: &Arc<AtomicBool>,
    emit: &Rc<dyn Fn(OperationEvent)>,
) -> glib::SourceId {
    let timer_progress = progress.clone();
    let timer_total = total.clone();
    let timer_cancelled = cancelled.clone();
    let timer_emit = emit.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        // Keep the source until the task calls remove(); Break would double-remove.
        if !timer_cancelled.load(Ordering::Relaxed) {
            timer_emit(OperationEvent::ArchiveProgress {
                request_id,
                completed: timer_progress.load(Ordering::Relaxed),
                total: timer_total.load(Ordering::Relaxed),
            });
        }
        glib::ControlFlow::Continue
    })
}

/// File extensions stored uncompressed by [`compress_zip`].
///
/// These formats are already compressed, so deflate spends CPU without shrinking
/// the archive.
const INCOMPRESSIBLE_EXTS: &[&str] = &[
    "zip", "7z", "gz", "bz2", "xz", "zst", "tar", "rar", "lz", "lz4", "br", "mp4", "mkv", "avi",
    "mov", "webm", "flv", "wmv", "jpg", "jpeg", "png", "webp", "gif", "heic", "avif", "bmp", "mp3",
    "flac", "aac", "ogg", "opus", "wma", "m4a", "pdf", "epub", "docx", "xlsx", "pptx", "odt",
    "ods", "odp", "iso", "dmg", "deb", "rpm", "apk", "jar", "war",
];

/// Returns whether `path`'s extension is in [`INCOMPRESSIBLE_EXTS`].
fn is_incompressible(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| INCOMPRESSIBLE_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Tracks renamed top-level entries so nested members follow the same rename.
///
/// If `docs` already exists in the destination, a member `docs/readme.txt`
/// is extracted under `docs (2)/readme.txt` rather than merging into `docs`.
struct ExtractNameResolver {
    renames: std::collections::HashMap<OsString, OsString>,
}

impl ExtractNameResolver {
    fn new() -> Self {
        Self {
            renames: std::collections::HashMap::new(),
        }
    }

    /// Maps a validated relative member path onto a conflict-free destination path.
    ///
    /// The top-level component is passed through [`ExtractionDestination::available_name`]
    /// once and remembered for later members that share that prefix.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` has no normal first component or
    /// [`ExtractionDestination::available_name`] fails.
    fn resolve(
        &mut self,
        destination: &ExtractionDestination,
        path: &Path,
    ) -> Result<PathBuf, String> {
        let top = path
            .components()
            .next()
            .and_then(|component| match component {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        let resolved_top = if let Some(existing) = self.renames.get(top) {
            existing.clone()
        } else {
            let name = destination.available_name(&destination.root, top)?;
            self.renames.insert(top.to_owned(), name.clone());
            name
        };
        let mut resolved = PathBuf::from(resolved_top);
        resolved.extend(
            path.components()
                .skip(1)
                .map(|component| component.as_os_str()),
        );
        Ok(resolved)
    }
}

/// Extracts every member of `archive` into `dest_dir`.
///
/// Member names are checked with [`zip::read::ZipFile::enclosed_name`] and
/// [`validated_archive_path`] before any create. Returns
/// [`ArchiveOutcome::Completed`] with the first created top-level name, or
/// [`ArchiveOutcome::Cancelled`] with remaining ZIP indexes listed as
/// not-attempted.
///
/// # Arguments
///
/// * `archive` - Open ZIP already positioned at the start of its members
/// * `dest_dir` - Directory that will be pinned as the extraction root
/// * `password` - Optional AES password for encrypted members
/// * `progress` - Counter incremented once per extracted member
/// * `cancelled` - Flag checked before each member and during copies
///
/// # Errors
///
/// - [`Failed`] if the destination cannot be opened, a member name is unsafe,
///   a member cannot be decrypted, or a create/copy fails for a reason other
///   than cancellation
///
/// Cancellation is returned as [`ArchiveOutcome::Cancelled`], not [`Cancelled`].
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn extract_zip_from_archive(
    archive: &mut zip::ZipArchive<std::fs::File>,
    dest_dir: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let destination = ExtractionDestination::open(dest_dir)?;
    let pw_bytes = password.map(|p| p.as_bytes());
    let mut resolver = ExtractNameResolver::new();
    let mut first_name = None;
    let mut completed = Vec::new();
    for i in 0..archive.len() {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(ArchiveOutcome::Cancelled {
                completed,
                failed: Vec::new(),
                not_attempted: zip_entry_locations(archive, dest_dir, i),
            });
        }
        let read_options = zip::read::ZipReadOptions::new().password(pw_bytes);
        let mut entry = archive
            .by_index_with_options(i, read_options)
            .map_err(archive_failed)?;
        let name = entry.name();
        entry
            .enclosed_name()
            .ok_or_else(|| format!("Refusing unsafe ZIP path: {name}"))?;
        let path = validated_archive_path(name)?;
        let outpath = resolver.resolve(&destination, &path)?;
        if first_name.is_none() {
            first_name = outpath
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().to_string());
        }
        if entry.is_dir() {
            destination.create_directories(&outpath)?;
        } else {
            let (mut outfile, created) = destination.create_file(&outpath)?;
            if let Err(error) = copy_with_big_buf(&mut entry, &mut outfile, cancelled) {
                drop(outfile);
                drop(entry);
                let removed = destination.remove_file(&created);
                return match error {
                    ArchiveError::Cancelled => Ok(cancelled_extract_after_partial_write(
                        dest_dir,
                        &created,
                        completed,
                        zip_entry_locations(archive, dest_dir, i + 1),
                        removed,
                    )),
                    failed => Err(failed),
                };
            }
        }
        completed.push(extract_entry_location(dest_dir, &outpath));
        progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(ArchiveOutcome::Completed(first_name))
}

/// Extracts a TAR or gzip-compressed TAR archive at `archive_path` into `dest_dir`.
///
/// TAR is a sequential format, so a cancel reports at most the current member
/// as not-attempted; later unread members are omitted from
/// [`CancelledOperation::not_attempted`].
///
/// # Arguments
///
/// * `archive_path` - Local path of the TAR file
/// * `dest_dir` - Directory that will be pinned as the extraction root
/// * `gzip` - Whether to wrap the file in a gzip decoder
/// * `progress` - Counter incremented once per extracted member
/// * `cancelled` - Flag checked before each member and during copies
///
/// # Errors
///
/// - [`Failed`] if the archive or destination cannot be opened, a member name
///   is unsafe, or a create/copy fails for a reason other than cancellation
///
/// Cancellation is returned as [`ArchiveOutcome::Cancelled`], not [`Cancelled`].
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn extract_tar(
    archive_path: &Path,
    dest_dir: &Path,
    gzip: bool,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let destination = ExtractionDestination::open(dest_dir)?;
    let file = std::fs::File::open(archive_path).map_err(archive_failed)?;
    let reader: Box<dyn std::io::Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    let mut resolver = ExtractNameResolver::new();
    let mut first_name = None;
    let mut completed = Vec::new();
    let entries = archive.entries().map_err(archive_failed)?;
    for entry in entries {
        if cancelled.load(Ordering::Relaxed) {
            let not_attempted = entry
                .ok()
                .and_then(|entry| tar_entry_location(entry, dest_dir))
                .into_iter()
                .collect();
            return Ok(ArchiveOutcome::Cancelled {
                completed,
                failed: Vec::new(),
                not_attempted,
            });
        }
        let mut entry = entry.map_err(archive_failed)?;
        let name = entry.path().map_err(archive_failed)?;
        if entry.header().entry_type().is_dir() && name == Path::new(".") {
            continue;
        }
        let path = validated_archive_path(&name.to_string_lossy())?;
        let outpath = resolver.resolve(&destination, &path)?;
        if first_name.is_none() {
            first_name = outpath
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().to_string());
        }
        if entry.header().entry_type().is_dir() {
            destination.create_directories(&outpath)?;
        } else {
            let (mut outfile, created) = destination.create_file(&outpath)?;
            if let Err(error) = copy_with_big_buf(&mut entry, &mut outfile, cancelled) {
                drop(outfile);
                drop(entry);
                let removed = destination.remove_file(&created);
                return match error {
                    ArchiveError::Cancelled => Ok(cancelled_extract_after_partial_write(
                        dest_dir,
                        &created,
                        completed,
                        Vec::new(),
                        removed,
                    )),
                    failed => Err(failed),
                };
            }
        }
        completed.push(extract_entry_location(dest_dir, &outpath));
        progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(ArchiveOutcome::Completed(first_name))
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
fn compress_7z(
    file: std::fs::File,
    entries: &[std::path::PathBuf],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, EncoderOptions, Lzma2Options};
    let mut writer = sevenz_rust2::ArchiveWriter::new(file).map_err(|e| e.to_string())?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let lzma2 =
        sevenz_rust2::EncoderConfiguration::new(sevenz_rust2::EncoderMethod::LZMA2).with_options(
            EncoderOptions::Lzma2(Lzma2Options::from_level_mt(6, threads, 1 << 26)),
        );
    if let Some(pw) = password {
        let methods = vec![lzma2, AesEncoderOptions::new(pw.into()).into()];
        writer.set_content_methods(methods);
    } else {
        writer.set_content_methods(vec![lzma2]);
    }
    visit_archive_entries(entries, cancelled, &mut |path, source| {
        let name = path.to_string_lossy();
        let (mut entry, file) = match source {
            ArchiveSource::Symlink(_) => {
                return Err(archive_failed(format!(
                    "7z compression does not support symbolic links: {}. Use ZIP or TAR instead.",
                    path.display()
                )));
            }
            ArchiveSource::Directory(file) => {
                (sevenz_rust2::ArchiveEntry::new_directory(&name), file)
            }
            ArchiveSource::File(file) => (sevenz_rust2::ArchiveEntry::new_file(&name), file),
        };
        let metadata = file.metadata().map_err(|error| error.to_string())?;
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
            Some(file)
        };
        writer
            .push_archive_entry(entry, reader)
            .map_err(archive_failed)?;
        check_archive_cancelled(cancelled)?;
        if reader.is_some() {
            progress.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    })?;
    check_archive_cancelled(cancelled)?;
    writer.finish().map_err(archive_failed)?;
    Ok(())
}

/// Extracts every member of a 7z archive from `reader` into `dest_dir`.
///
/// Cancellation is expressed by returning [`sevenz_cancelled`] from the
/// `sevenz_rust2` entry callback, then mapped back to
/// [`ArchiveOutcome::Cancelled`]. Member names are still filtered through
/// [`validated_archive_path`].
///
/// # Arguments
///
/// * `reader` - Open 7z file
/// * `dest_dir` - Directory that will be pinned as the extraction root
/// * `password` - Password handle required by `sevenz_rust2`, empty when unused
/// * `progress` - Counter incremented once per extracted member
/// * `cancelled` - Flag checked before each member and during copies
///
/// # Errors
///
/// - [`Failed`] if the archive or destination cannot be opened, a member name
///   is unsafe, or a create/copy fails for a reason other than cancellation
///
/// Cancellation is returned as [`ArchiveOutcome::Cancelled`], not [`Cancelled`].
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn extract_7z_from_reader(
    reader: std::fs::File,
    dest_dir: &Path,
    password: sevenz_rust2::Password,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let destination = ExtractionDestination::open(dest_dir)?;
    let mut archive = sevenz_rust2::ArchiveReader::new(reader, password).map_err(archive_failed)?;
    let entry_names: Vec<String> = archive
        .archive()
        .files
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let resolver = RefCell::new(ExtractNameResolver::new());
    let first_name = RefCell::new(None::<String>);
    let completed = RefCell::new(Vec::new());
    let failed = RefCell::new(Vec::new());
    let not_attempted = RefCell::new(Vec::new());
    let progress = progress.clone();
    let dest_dir = dest_dir.to_path_buf();
    let result = archive.for_each_entries(|entry, reader| {
        if cancelled.load(Ordering::Relaxed) {
            *not_attempted.borrow_mut() =
                sevenz_locations_from(&dest_dir, &entry_names, &entry.name, false);
            return Err(sevenz_cancelled());
        }
        let path = validated_archive_path(&entry.name)
            .map_err(|error| sevenz_rust2::Error::Other(error.into()))?;
        let outpath = resolver
            .borrow_mut()
            .resolve(&destination, &path)
            .map_err(|error| sevenz_rust2::Error::Other(error.into()))?;
        if first_name.borrow().is_none() {
            *first_name.borrow_mut() = outpath
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().to_string());
        }
        if entry.is_directory {
            destination
                .create_directories(&outpath)
                .map_err(|error| sevenz_rust2::Error::Other(error.into()))?;
        } else {
            let (mut file, created) = destination
                .create_file(&outpath)
                .map_err(|error| sevenz_rust2::Error::Other(error.into()))?;
            if let Err(error) = copy_with_big_buf(reader, &mut file, cancelled) {
                drop(file);
                let removed = destination.remove_file(&created);
                return match error {
                    ArchiveError::Cancelled => {
                        if removed.is_err() {
                            failed
                                .borrow_mut()
                                .push(extract_entry_location(&dest_dir, &created));
                            *not_attempted.borrow_mut() =
                                sevenz_locations_from(&dest_dir, &entry_names, &entry.name, true);
                        } else {
                            let mut remaining = vec![extract_entry_location(&dest_dir, &created)];
                            remaining.extend(sevenz_locations_from(
                                &dest_dir,
                                &entry_names,
                                &entry.name,
                                true,
                            ));
                            *not_attempted.borrow_mut() = remaining;
                        }
                        Err(sevenz_cancelled())
                    }
                    ArchiveError::Failed(message) => {
                        Err(sevenz_rust2::Error::Other(message.into()))
                    }
                };
            }
        }
        completed
            .borrow_mut()
            .push(extract_entry_location(&dest_dir, &outpath));
        progress.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    });
    match result {
        Ok(()) => Ok(ArchiveOutcome::Completed(first_name.into_inner())),
        Err(error) if sevenz_is_cancelled(&error) => Ok(ArchiveOutcome::Cancelled {
            completed: completed.into_inner(),
            failed: failed.into_inner(),
            not_attempted: not_attempted.into_inner(),
        }),
        Err(error) => Err(archive_failed(error)),
    }
}
