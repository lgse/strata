// SPDX-License-Identifier: GPL-3.0-or-later

//! Staged archive publication and the existing format-specific writers.

#[cfg(test)]
mod tests;

use super::super::{
    local_directory_children, open_local_child_directory, open_local_parent_directory,
};
use super::{ArchiveError, COPY_BUF, archive_failed, check_archive_cancelled, copy_with_big_buf};
use crate::services::TransferConflict;
use gtk::gio;
use std::{
    ffi::{OsStr, OsString},
    io,
    os::{
        fd::AsFd,
        unix::{ffi::OsStringExt, fs::PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
pub(super) async fn write_staged_archive<F>(
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
pub(super) fn compress_zip(
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
pub(super) fn compress_tar(
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
pub(super) fn count_archive_files(
    entries: &[PathBuf],
    cancelled: &AtomicBool,
) -> Result<usize, ArchiveError> {
    let mut count = 0;
    visit_archive_entries(entries, cancelled, &mut |_, source| {
        if !matches!(source, ArchiveSource::Directory(_)) {
            count += 1;
        }
        Ok(())
    })?;
    Ok(count)
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
