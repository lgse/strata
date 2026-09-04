// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    future::Future,
    io,
    os::{
        fd::{AsFd, OwnedFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::PermissionsExt,
        },
    },
    path::{Component, Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::location_for_file,
    model::Location,
    services::{
        ArchiveFormat, CancelledOperation, CompressRequest, CreateDirectoryRequest,
        CreateFileRequest, DeleteRequest, ExtractRequest, LoadHandle, OperationEvent,
        OperationProvider, OperationRequestId, PasteRequest, RenameRequest, RestoreRequest,
        RestoreSource, TransferConflict, validate_basename,
    },
};

async fn await_cancellable<O, T>(
    object: &O,
    cancellable: &gio::Cancellable,
    start: impl FnOnce(&O, &gio::Cancellable, gio::GioFutureResult<Result<T, glib::Error>>) + 'static,
) -> Result<T, glib::Error>
where
    O: Clone + 'static,
    T: 'static,
{
    // The backend's callback is authoritative: cancellation can race with a successful result.
    let cancellable = cancellable.clone();
    gio::GioFuture::new(object, move |object, _, result| {
        start(object, &cancellable, result);
    })
    .await
}

fn gio_file(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}

fn validated_child(parent: &gio::File, name: &str) -> Result<gio::File, &'static str> {
    validate_basename(name)?;
    Ok(parent.child(name))
}

fn transfer_is_noop(source: &gio::File, destination: &gio::File, target: &gio::File) -> bool {
    source.equal(target) || source.equal(destination) || destination.has_prefix(source)
}

fn was_cancelled(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled)
}

/// Whether a delete failure is retryable as a permanent delete: it was a
/// trash attempt (never a permanent one, which has no further fallback),
/// and the destination doesn't support Trash at all rather than some other,
/// unrelated failure.
fn is_trash_unsupported_failure(permanent: bool, error: &glib::Error) -> bool {
    !permanent && error.matches(gio::IOErrorEnum::NotSupported)
}

fn copy_recursively(
    source: gio::File,
    target: gio::File,
    overwrite_existing: bool,
    cancellable: gio::Cancellable,
    created_root: Option<Rc<Cell<bool>>>,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let info = await_cancellable(&source, &cancellable, |source, cancellable, result| {
            source.query_info_async(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        if info.file_type() == gio::FileType::Directory {
            if !overwrite_existing || !target.query_exists(Some(&cancellable)) {
                await_cancellable(&target, &cancellable, |target, cancellable, result| {
                    target.make_directory_async(
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                })
                .await?;
                if let Some(created_root) = &created_root {
                    created_root.set(true);
                }
            }
            let enumerator =
                await_cancellable(&source, &cancellable, |source, cancellable, result| {
                    source.enumerate_children_async(
                        "standard::name",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                })
                .await?;
            loop {
                let children = await_cancellable(
                    &enumerator,
                    &cancellable,
                    |enumerator, cancellable, result| {
                        enumerator.next_files_async(
                            64,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    },
                )
                .await?;
                if children.is_empty() {
                    break;
                }
                for child in children {
                    copy_recursively(
                        source.child(child.name()),
                        target.child(child.name()),
                        overwrite_existing,
                        cancellable.clone(),
                        None,
                    )
                    .await?;
                }
            }
            Ok(())
        } else {
            let flags = gio::FileCopyFlags::ALL_METADATA
                | gio::FileCopyFlags::NOFOLLOW_SYMLINKS
                | if overwrite_existing {
                    gio::FileCopyFlags::OVERWRITE
                } else {
                    gio::FileCopyFlags::NONE
                };
            await_cancellable(&source, &cancellable, move |source, cancellable, result| {
                source.copy_async(
                    &target,
                    flags,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    None,
                    move |output| result.resolve(output),
                );
            })
            .await
        }
    })
}

async fn copy_new_recursively(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
) -> Result<(), glib::Error> {
    if source.is_native()
        && target.is_native()
        && let Some(target_path) = target.path()
    {
        let source_type =
            await_cancellable(&source, &cancellable, |source, cancellable, result| {
                source.query_info_async(
                    "standard::type",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |output| result.resolve(output),
                );
            })
            .await?
            .file_type();
        if source_type == gio::FileType::Directory {
            let parent = target_path
                .parent()
                .ok_or_else(|| io_error("The destination has no parent directory"))?;
            let staged = StagedSibling::create(parent, true).map_err(io_error)?;
            if let Err(error) = copy_recursively(
                source,
                gio::File::for_path(staged.path()),
                true,
                cancellable.clone(),
                None,
            )
            .await
            {
                discard_staged(staged).await;
                return Err(error);
            }
            if let Err(error) = cancellable.set_error_if_cancelled() {
                discard_staged(staged).await;
                return Err(error);
            }

            let staged_path = staged.path().to_owned();
            let committed = gio::spawn_blocking(move || {
                rustix::fs::renameat_with(
                    rustix::fs::CWD,
                    &staged_path,
                    rustix::fs::CWD,
                    &target_path,
                    rustix::fs::RenameFlags::NOREPLACE,
                )
            })
            .await
            .map_err(|_| io_error("The copy worker stopped unexpectedly"));
            let committed = match committed {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(io_error(format!(
                    "Could not finish copying the item: {error}"
                ))),
                Err(error) => Err(error),
            };
            if let Err(error) = committed {
                discard_staged(staged).await;
                return Err(error);
            }
            return Ok(());
        }
    }

    let created_root = Rc::new(Cell::new(false));
    let result = copy_recursively(
        source,
        target.clone(),
        false,
        cancellable.clone(),
        Some(created_root.clone()),
    )
    .await;
    if result.as_ref().is_err_and(was_cancelled) && created_root.get() {
        let cleanup = gio::Cancellable::new();
        let _cleanup_result = permanently_delete(target, true, cleanup).await;
    }
    result
}

type MoveAttempt = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

async fn move_local_with(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    attempt_move: MoveAttempt,
) -> Result<(), glib::Error> {
    let result = attempt_move(source.clone(), target.clone(), cancellable.clone()).await;
    match result {
        Err(error) if error.matches(gio::IOErrorEnum::WouldRecurse) => {
            copy_new_recursively(source.clone(), target, cancellable.clone()).await?;
            permanently_delete(source, true, cancellable).await
        }
        other => other,
    }
}

async fn move_local(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
) -> Result<(), glib::Error> {
    move_local_with(
        source,
        target,
        cancellable,
        Rc::new(|source, target, cancellable| {
            Box::pin(async move {
                let flags =
                    gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
                await_cancellable(&source, &cancellable, move |source, cancellable, result| {
                    source.move_async(
                        &target,
                        flags,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        None,
                        move |output| result.resolve(output),
                    );
                })
                .await
            })
        }),
    )
    .await
}

enum StagedSibling {
    File(tempfile::TempPath),
    Directory(tempfile::TempDir),
}

impl StagedSibling {
    fn create(parent: &Path, directory: bool) -> io::Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(".strata-replacement-");
        if directory {
            builder.tempdir_in(parent).map(Self::Directory)
        } else {
            builder
                .tempfile_in(parent)
                .map(tempfile::NamedTempFile::into_temp_path)
                .map(Self::File)
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Directory(directory) => directory.path(),
        }
    }
}

async fn discard_staged(staged: StagedSibling) {
    let _discarded = gio::spawn_blocking(move || drop(staged)).await;
}

fn io_error(error: impl std::fmt::Display) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string())
}

type StageCopy = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        bool,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

async fn replace_local_with(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    copy_to_stage: StageCopy,
) -> Result<(), glib::Error> {
    if let Some(locations) = affected_locations {
        locations.extend([&source, &target].into_iter().filter_map(location_for_file));
    }
    if source.path().is_none() {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "Safe replacement is unavailable for this source",
        ));
    }
    let target_path = target.path().ok_or_else(|| {
        glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "Safe replacement is unavailable at this destination",
        )
    })?;
    let parent = target_path
        .parent()
        .ok_or_else(|| io_error("The destination has no parent directory"))?;
    let source_type = await_cancellable(&source, &cancellable, |source, cancellable, result| {
        source.query_info_async(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?
    .file_type();
    let target_type = await_cancellable(&target, &cancellable, |target, cancellable, result| {
        target.query_info_async(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?
    .file_type();
    let source_is_directory = source_type == gio::FileType::Directory;
    let target_is_directory = target_type == gio::FileType::Directory;
    if source_is_directory != target_is_directory {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "A file and a folder cannot safely replace one another",
        ));
    }

    let staged = StagedSibling::create(parent, source_is_directory).map_err(io_error)?;
    let staged_file = gio::File::for_path(staged.path());
    if let Err(error) = copy_to_stage(
        source.clone(),
        staged_file.clone(),
        source_is_directory,
        cancellable.clone(),
    )
    .await
    {
        discard_staged(staged).await;
        return Err(error);
    }
    if let Err(error) = cancellable.set_error_if_cancelled() {
        discard_staged(staged).await;
        return Err(error);
    }

    let staged_path = staged.path().to_owned();
    let exchanged = gio::spawn_blocking(move || {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            &staged_path,
            rustix::fs::CWD,
            &target_path,
            rustix::fs::RenameFlags::EXCHANGE,
        )
    })
    .await
    .map_err(|_| io_error("The replacement worker stopped unexpectedly"));
    let exchanged = match exchanged {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(io_error(format!(
            "Could not safely replace the item: {error}"
        ))),
        Err(error) => Err(error),
    };
    if let Err(error) = exchanged {
        discard_staged(staged).await;
        return Err(error);
    }

    if let Err(error) =
        permanently_delete(staged_file, target_is_directory, gio::Cancellable::new()).await
    {
        discard_staged(staged).await;
        return Err(error);
    }
    if move_source {
        permanently_delete(source, source_is_directory, cancellable).await?;
    }
    Ok(())
}

async fn replace_local(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
) -> Result<(), glib::Error> {
    replace_local_with(
        source,
        target,
        move_source,
        cancellable,
        affected_locations,
        Rc::new(|source, staged, directory, cancellable| {
            Box::pin(async move {
                if directory {
                    copy_recursively(source, staged, true, cancellable, None).await
                } else {
                    let flags = gio::FileCopyFlags::ALL_METADATA
                        | gio::FileCopyFlags::NOFOLLOW_SYMLINKS
                        | gio::FileCopyFlags::OVERWRITE;
                    await_cancellable(&source, &cancellable, move |source, cancellable, result| {
                        source.copy_async(
                            &staged,
                            flags,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            None,
                            move |output| result.resolve(output),
                        );
                    })
                    .await
                }
            })
        }),
    )
    .await
}

fn permanently_delete(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if directory {
            let enumerator = await_cancellable(&file, &cancellable, |file, cancellable, result| {
                file.enumerate_children_async(
                    "standard::name,standard::type",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |output| result.resolve(output),
                );
            })
            .await?;
            loop {
                let children = await_cancellable(
                    &enumerator,
                    &cancellable,
                    |enumerator, cancellable, result| {
                        enumerator.next_files_async(
                            64,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    },
                )
                .await?;
                if children.is_empty() {
                    break;
                }
                for child in children {
                    permanently_delete(
                        file.child(child.name()),
                        child.file_type() == gio::FileType::Directory,
                        cancellable.clone(),
                    )
                    .await?;
                }
            }
        }
        await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.delete_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
                result.resolve(output)
            });
        })
        .await
    })
}

/// The outcome of resolving one delete target relative to its parent
/// directory's file descriptor.
enum LocalDeleteStep {
    /// A non-directory entry (file, symlink, or other special file) that has
    /// already been unlinked.
    Removed,
    /// A directory that was opened (not yet removed) along with its
    /// immediate children, still to be deleted before the directory itself.
    Directory {
        handle: OwnedFd,
        children: Vec<OsString>,
    },
}

/// Inspects and, for non-directories, immediately deletes the entry named
/// `name` inside `parent`. The type is re-read from disk here rather than
/// trusted from any earlier listing, so a symlink swapped in for a
/// directory is deleted as the symlink it now is instead of being opened as
/// a directory.
fn open_local_delete_target<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
) -> Result<LocalDeleteStep, String> {
    let stat = rustix::fs::statat(parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("Could not inspect {}: {error}", name.to_string_lossy()))?;
    if !matches!(
        rustix::fs::FileType::from_raw_mode(stat.st_mode),
        rustix::fs::FileType::Directory
    ) {
        rustix::fs::unlinkat(parent, name, rustix::fs::AtFlags::empty())
            .map_err(|error| format!("Could not delete {}: {error}", name.to_string_lossy()))?;
        return Ok(LocalDeleteStep::Removed);
    }
    // RESOLVE_NO_SYMLINKS (stronger than O_NOFOLLOW) plus RESOLVE_BENEATH and
    // RESOLVE_NO_MAGICLINKS: if `name` changed to a symlink (or a magic link)
    // in the moment since the statat above, this fails closed instead of
    // opening whatever it now points to.
    let handle = rustix::fs::openat2(
        parent,
        name,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH
            | rustix::fs::ResolveFlags::NO_SYMLINKS
            | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|error| {
        format!(
            "{} changed while it was being deleted: {error}",
            name.to_string_lossy()
        )
    })?;
    let mut children = Vec::new();
    for entry in rustix::fs::Dir::read_from(&handle).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let entry_name = entry.file_name();
        if entry_name == c"." || entry_name == c".." {
            continue;
        }
        children.push(OsString::from_vec(entry_name.to_bytes().to_vec()));
    }
    Ok(LocalDeleteStep::Directory { handle, children })
}

fn cancelled_local_delete() -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Cancelled, "Delete cancelled")
}

async fn run_local_delete_step<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, glib::Error> {
    gio::spawn_blocking(work)
        .await
        .map_err(|_| io_error("Delete task panicked"))?
        .map_err(io_error)
}

/// Recursively and permanently deletes the entry named `name` inside
/// `parent`, walking descriptor-relative to each already-open directory
/// rather than re-resolving paths, so a component swapped out from under an
/// in-progress delete cannot redirect it outside the tree it started in.
fn permanently_delete_local(
    parent: OwnedFd,
    name: OsString,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if cancellable.is_cancelled() {
            return Err(cancelled_local_delete());
        }
        let step_parent = parent.try_clone().map_err(io_error)?;
        let step_name = name.clone();
        let step =
            run_local_delete_step(move || open_local_delete_target(&step_parent, &step_name))
                .await?;
        let LocalDeleteStep::Directory { handle, children } = step else {
            return Ok(());
        };
        for child in children {
            if cancellable.is_cancelled() {
                return Err(cancelled_local_delete());
            }
            let child_parent = handle.try_clone().map_err(io_error)?;
            permanently_delete_local(child_parent, child, cancellable.clone()).await?;
        }
        run_local_delete_step(move || {
            rustix::fs::unlinkat(&parent, &name, rustix::fs::AtFlags::REMOVEDIR)
                .map_err(|error| format!("Could not delete {}: {error}", name.to_string_lossy()))
        })
        .await
    })
}

/// Entry point for permanently deleting a local path: opens the target's
/// parent directory once, then hands off to the descriptor-relative walk in
/// [`permanently_delete_local`] for everything below it.
fn permanently_delete_local_path(
    path: PathBuf,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let Some(parent_path) = path.parent().map(Path::to_path_buf) else {
            return Err(io_error("Cannot permanently delete the filesystem root"));
        };
        let Some(name) = path.file_name().map(OsStr::to_os_string) else {
            return Err(io_error("Invalid delete target"));
        };
        let parent = run_local_delete_step(move || {
            rustix::fs::open(
                &parent_path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|error| format!("Could not open {}: {error}", parent_path.display()))
        })
        .await?;
        permanently_delete_local(parent, name, cancellable).await
    })
}

fn operation_error_summary(errors: &[String], action: &str) -> String {
    let mut summary = format!(
        "{} could not be {action}. The remaining items were processed.",
        if errors.len() == 1 {
            "1 item".to_owned()
        } else {
            format!("{} items", errors.len())
        }
    );
    for error in errors.iter().take(8) {
        summary.push_str("\n\n• ");
        summary.push_str(error);
    }
    if errors.len() > 8 {
        summary.push_str(&format!("\n\n…and {} more", errors.len() - 8));
    }
    summary
}

async fn write_staged_archive<F>(
    destination: &Path,
    archive_path: &Path,
    conflict: TransferConflict,
    write_archive: F,
) -> Result<(), String>
where
    F: FnOnce(std::fs::File) -> Result<(), String> + Send + 'static,
{
    let existing_permissions = if conflict == TransferConflict::ReplaceExisting {
        match std::fs::symlink_metadata(archive_path) {
            Ok(metadata) if metadata.file_type().is_file() => Some(metadata.permissions()),
            Ok(_) => None,
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        }
    } else {
        None
    };
    let mut builder = tempfile::Builder::new();
    builder
        .prefix(".strata-compression-")
        .permissions(std::fs::Permissions::from_mode(0o666));
    let staged = builder
        .tempfile_in(destination)
        .map_err(|error| error.to_string())?;
    let file = staged.reopen().map_err(|error| error.to_string())?;
    gio::spawn_blocking(move || write_archive(file))
        .await
        .map_err(|_| "Compression task panicked".to_owned())??;
    if let Some(permissions) = existing_permissions {
        staged
            .as_file()
            .set_permissions(permissions)
            .map_err(|error| error.to_string())?;
    }
    match conflict {
        TransferConflict::FailIfExists => staged.persist_noclobber(archive_path),
        TransferConflict::ReplaceExisting => staged.persist(archive_path),
    }
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn deletion_error_summary(errors: &[String]) -> String {
    operation_error_summary(errors, "deleted")
}

/// Backends without Trash support (most remote filesystems, including SMB)
/// fail a move-to-trash with `NOT_SUPPORTED`. Give an actionable message for
/// that specific case instead of the raw GIO error text.
fn deletion_error_message(name: &str, permanent: bool, error: &glib::Error) -> String {
    if !permanent && error.matches(gio::IOErrorEnum::NotSupported) {
        format!("{name}: This location doesn't support Trash. Delete permanently instead.")
    } else {
        format!("{name}: {error}")
    }
}

#[derive(Clone)]
struct RestoreEntry {
    source: Location,
    display_name: String,
    original_target: Option<Location>,
    trash_info: Option<PathBuf>,
}

async fn trashed_entries_for_originals(
    original_locations: &[Location],
) -> Result<Vec<RestoreEntry>, glib::Error> {
    let requested = original_locations
        .iter()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect::<HashSet<_>>();
    // GVfs can miss an item re-trashed under the same basename after a restore, so prefer the
    // authoritative freedesktop.org metadata for the home trash before consulting trash:///.
    let fallback_requested = requested.clone();
    let mut fallback = gio::spawn_blocking(move || home_trash_entries(&fallback_requested))
        .await
        .map_err(|_| glib::Error::new(gio::IOErrorEnum::Failed, "Trash lookup task failed"))?;
    if fallback.len() == requested.len() && requested.len() == original_locations.len() {
        return Ok(original_locations
            .iter()
            .filter_map(|location| location.native_path())
            .filter_map(|path| fallback.remove(path))
            .collect());
    }

    let trash = gio::File::for_uri("trash:///");
    let enumerator = trash
        .enumerate_children_future(
            "standard::name,standard::display-name,trash::orig-path,trash::deletion-date",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut newest = HashMap::<PathBuf, (String, Location, String)>::new();
    loop {
        let infos = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if infos.is_empty() {
            break;
        }
        for info in infos {
            let Some(original_path) = info.attribute_byte_string("trash::orig-path") else {
                continue;
            };
            let original_path = PathBuf::from(original_path.as_str());
            if !requested.contains(&original_path) {
                continue;
            }
            let deletion_date = info
                .attribute_string("trash::deletion-date")
                .map(|value| value.to_string())
                .unwrap_or_default();
            let Some(location) = location_for_file(&trash.child(info.name())) else {
                continue;
            };
            let candidate = (deletion_date, location, info.display_name().to_string());
            match newest.get(&original_path) {
                Some(current) if current.0 >= candidate.0 => {}
                _ => {
                    newest.insert(original_path, candidate);
                }
            }
        }
    }

    if requested
        .iter()
        .any(|path| !fallback.contains_key(path) && !newest.contains_key(path))
        || requested.len() != original_locations.len()
    {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotFound,
            "One or more recently trashed items are no longer available",
        ));
    }
    Ok(original_locations
        .iter()
        .filter_map(|location| location.native_path())
        .filter_map(|path| {
            fallback.remove(path).or_else(|| {
                newest
                    .remove(path)
                    .map(|(_, source, display_name)| RestoreEntry {
                        source,
                        display_name,
                        original_target: None,
                        trash_info: None,
                    })
            })
        })
        .collect())
}

fn home_trash_entries(requested: &HashSet<PathBuf>) -> HashMap<PathBuf, RestoreEntry> {
    home_trash_entries_at(&glib::user_data_dir().join("Trash"), requested)
}

fn home_trash_entries_at(
    trash_root: &Path,
    requested: &HashSet<PathBuf>,
) -> HashMap<PathBuf, RestoreEntry> {
    let info_root = trash_root.join("info");
    let files_root = trash_root.join("files");
    let mut newest = HashMap::<PathBuf, (String, RestoreEntry)>::new();
    let Ok(infos) = std::fs::read_dir(info_root) else {
        return HashMap::new();
    };
    for info in infos.flatten() {
        let info_path = info.path();
        let Some(name) = info_path.file_name() else {
            continue;
        };
        let bytes = name.as_bytes();
        let Some(file_name) = bytes.strip_suffix(b".trashinfo") else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(&info_path) else {
            continue;
        };
        let encoded_path = contents.lines().find_map(|line| line.strip_prefix("Path="));
        let deletion_date = contents
            .lines()
            .find_map(|line| line.strip_prefix("DeletionDate="))
            .unwrap_or_default();
        let Some(original_path) =
            encoded_path.and_then(|path| gio::File::for_uri(&format!("file://{path}")).path())
        else {
            continue;
        };
        if !requested.contains(&original_path) {
            continue;
        }
        let source_path = files_root.join(OsString::from_vec(file_name.to_vec()));
        if std::fs::symlink_metadata(&source_path).is_err() {
            continue;
        }
        let entry = RestoreEntry {
            source: Location::local(&source_path),
            display_name: source_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Trashed item".to_owned()),
            original_target: Some(Location::local(&original_path)),
            trash_info: Some(info_path),
        };
        match newest.get(&original_path) {
            Some((current_date, _)) if current_date.as_str() >= deletion_date => {}
            _ => {
                newest.insert(original_path, (deletion_date.to_owned(), entry));
            }
        }
    }
    newest
        .into_iter()
        .map(|(path, (_, entry))| (path, entry))
        .collect()
}

fn cancellation_handle(cancellable: gio::Cancellable) -> LoadHandle {
    LoadHandle::new(move || cancellable.cancel())
}

fn cancelled_event(
    request_id: crate::services::OperationRequestId,
    completed: Vec<Location>,
    failed: Vec<Location>,
    not_attempted: Vec<Location>,
    affected_locations: HashSet<Location>,
) -> OperationEvent {
    OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed,
            failed,
            not_attempted,
            affected_locations,
        },
    }
}

#[derive(Default)]
pub struct LocalOperationProvider;

impl OperationProvider for LocalOperationProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            if let Err(message) = validate_basename(&request.new_name) {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: message.to_owned(),
                });
                return;
            }
            let file = request
                .entry
                .location
                .native_path()
                .map(gio::File::for_path)
                .unwrap_or_else(|| {
                    gio::File::for_uri(request.entry.location.uri_value().unwrap_or_default())
                });
            let item = request.entry.location.clone();
            let affected_locations = item.parent().into_iter().collect();
            if operation_cancellable.is_cancelled() {
                emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    Vec::new(),
                    vec![item],
                    affected_locations,
                ));
                return;
            }
            match await_cancellable(
                &file,
                &operation_cancellable,
                move |file, cancellable, result| {
                    file.set_display_name_async(
                        &request.new_name,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                },
            )
            .await
            {
                Ok(_) => emit(OperationEvent::Renamed {
                    request_id: request.id,
                }),
                Err(error) if was_cancelled(&error) => {
                    emit(cancelled_event(
                        request.id,
                        Vec::new(),
                        vec![item],
                        Vec::new(),
                        affected_locations,
                    ));
                }
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.to_string(),
                }),
            }
        });
        cancellation_handle(cancellable)
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let parent = gio_file(&request.parent);
            let folder = match validated_child(&parent, &request.name) {
                Ok(folder) => folder,
                Err(message) => {
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: message.to_owned(),
                    });
                    return;
                }
            };
            let Some(item) = location_for_file(&folder) else {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: "The new folder has an invalid URI".to_owned(),
                });
                return;
            };
            let affected_locations = HashSet::from([request.parent.clone()]);
            if operation_cancellable.is_cancelled() {
                emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    Vec::new(),
                    vec![item],
                    affected_locations,
                ));
                return;
            }
            match await_cancellable(
                &folder,
                &operation_cancellable,
                |folder, cancellable, result| {
                    folder.make_directory_async(
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                },
            )
            .await
            {
                Ok(()) => emit(OperationEvent::Created {
                    request_id: request.id,
                }),
                Err(error) if was_cancelled(&error) => {
                    emit(cancelled_event(
                        request.id,
                        Vec::new(),
                        vec![item],
                        Vec::new(),
                        affected_locations,
                    ));
                }
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.to_string(),
                }),
            }
        });
        cancellation_handle(cancellable)
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let parent = gio_file(&request.parent);
            let file = match validated_child(&parent, &request.name) {
                Ok(file) => file,
                Err(message) => {
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: message.to_owned(),
                    });
                    return;
                }
            };
            let Some(item) = location_for_file(&file) else {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: "The new file has an invalid URI".to_owned(),
                });
                return;
            };
            let affected_locations = HashSet::from([request.parent.clone()]);
            if operation_cancellable.is_cancelled() {
                emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    Vec::new(),
                    vec![item],
                    affected_locations,
                ));
                return;
            }
            match await_cancellable(
                &file,
                &operation_cancellable,
                |file, cancellable, result| {
                    file.create_async(
                        gio::FileCreateFlags::NONE,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                },
            )
            .await
            {
                Ok(_) => emit(OperationEvent::Created {
                    request_id: request.id,
                }),
                Err(error) if was_cancelled(&error) => {
                    emit(cancelled_event(
                        request.id,
                        Vec::new(),
                        vec![item],
                        Vec::new(),
                        affected_locations,
                    ));
                }
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.to_string(),
                }),
            }
        });
        cancellation_handle(cancellable)
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let destination = gio_file(&request.destination);
            let mut affected_locations = HashSet::from([request.destination.clone()]);
            for parent in request.items.iter().filter_map(|item| item.source.parent()) {
                affected_locations.insert(parent);
            }
            let total = request.items.len();
            let mut completed = Vec::new();
            for (index, item) in request.items.iter().enumerate() {
                if operation_cancellable.is_cancelled() {
                    emit(cancelled_event(
                        request.id,
                        completed,
                        Vec::new(),
                        request.items[index..]
                            .iter()
                            .map(|item| item.source.clone())
                            .collect(),
                        affected_locations,
                    ));
                    return;
                }
                let source = gio_file(&item.source);
                let Some(name) = source.basename() else {
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: "A clipboard item has no file name".to_owned(),
                    });
                    return;
                };
                let target = destination.child(name);
                if transfer_is_noop(&source, &destination, &target) {
                    completed.push(item.source.clone());
                    emit(OperationEvent::TransferProgress {
                        request_id: request.id,
                        completed: completed.len(),
                        total,
                    });
                    continue;
                }
                affected_locations.insert(item.source.clone());
                if let Some(target) = location_for_file(&target) {
                    affected_locations.insert(target);
                }
                let result = if item.conflict == TransferConflict::ReplaceExisting {
                    replace_local(
                        source,
                        target,
                        request.move_sources,
                        operation_cancellable.clone(),
                        Some(&mut affected_locations),
                    )
                    .await
                } else if request.move_sources {
                    move_local(source, target, operation_cancellable.clone()).await
                } else {
                    copy_new_recursively(source, target, operation_cancellable.clone()).await
                };
                if let Err(error) = result {
                    if was_cancelled(&error) {
                        emit(cancelled_event(
                            request.id,
                            completed,
                            vec![item.source.clone()],
                            request.items[index + 1..]
                                .iter()
                                .map(|item| item.source.clone())
                                .collect(),
                            affected_locations,
                        ));
                        return;
                    }
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: completed,
                        message: error.to_string(),
                    });
                    return;
                }
                completed.push(item.source.clone());
                emit(OperationEvent::TransferProgress {
                    request_id: request.id,
                    completed: completed.len(),
                    total,
                });
            }
            emit(OperationEvent::Pasted {
                request_id: request.id,
                locations: completed,
            });
        });
        cancellation_handle(cancellable)
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let mut errors = Vec::new();
            let mut deleted_locations = Vec::new();
            let mut failed_locations = Vec::new();
            let mut retryable_locations = Vec::new();
            let mut affected_locations = HashSet::new();
            for entry in &request.entries {
                if let Some(parent) = entry.location.parent() {
                    affected_locations.insert(parent);
                }
                if entry.is_directory() {
                    affected_locations.insert(entry.location.clone());
                }
            }
            if !request.permanent {
                affected_locations.insert(Location::uri("trash:///"));
            }
            let total = request.entries.len();
            for (index, entry) in request.entries.iter().enumerate() {
                if operation_cancellable.is_cancelled() {
                    emit(cancelled_event(
                        request.id,
                        deleted_locations,
                        failed_locations,
                        request.entries[index..]
                            .iter()
                            .map(|entry| entry.location.clone())
                            .collect(),
                        affected_locations,
                    ));
                    return;
                }
                let file = gio_file(&entry.location);
                let result = if request.permanent {
                    if entry
                        .location
                        .uri_value()
                        .is_some_and(|uri| uri.starts_with("trash:"))
                    {
                        await_cancellable(
                            &file,
                            &operation_cancellable,
                            |file, cancellable, result| {
                                file.delete_async(
                                    glib::Priority::DEFAULT,
                                    Some(cancellable),
                                    move |output| result.resolve(output),
                                );
                            },
                        )
                        .await
                    } else if let Some(native_path) = entry.location.native_path() {
                        permanently_delete_local_path(
                            native_path.to_path_buf(),
                            operation_cancellable.clone(),
                        )
                        .await
                    } else {
                        // Remote (GVfs) locations have no local file descriptor to
                        // walk against, so this falls back to the path-based
                        // GIO delete rather than claiming an equivalent guarantee.
                        permanently_delete(
                            file,
                            entry.is_directory(),
                            operation_cancellable.clone(),
                        )
                        .await
                    }
                } else {
                    await_cancellable(
                        &file,
                        &operation_cancellable,
                        |file, cancellable, result| {
                            file.trash_async(
                                glib::Priority::DEFAULT,
                                Some(cancellable),
                                move |output| result.resolve(output),
                            );
                        },
                    )
                    .await
                };
                let deleted_location = if let Err(error) = result {
                    if was_cancelled(&error) {
                        failed_locations.push(entry.location.clone());
                        emit(cancelled_event(
                            request.id,
                            deleted_locations,
                            failed_locations,
                            request.entries[index + 1..]
                                .iter()
                                .map(|entry| entry.location.clone())
                                .collect(),
                            affected_locations,
                        ));
                        return;
                    }
                    if is_trash_unsupported_failure(request.permanent, &error) {
                        retryable_locations.push(entry.location.clone());
                    }
                    errors.push(deletion_error_message(
                        &entry.display_name,
                        request.permanent,
                        &error,
                    ));
                    failed_locations.push(entry.location.clone());
                    None
                } else {
                    deleted_locations.push(entry.location.clone());
                    Some(entry.location.clone())
                };
                emit(OperationEvent::DeleteProgress {
                    request_id: request.id,
                    completed: index + 1,
                    total,
                    deleted_location,
                });
            }
            if errors.is_empty() {
                emit(OperationEvent::Deleted {
                    request_id: request.id,
                    locations: deleted_locations,
                });
            } else {
                let has_non_retryable_failures = errors.len() > retryable_locations.len();
                emit(OperationEvent::CompletedWithErrors {
                    request_id: request.id,
                    deleted_locations,
                    retryable_locations,
                    has_non_retryable_failures,
                    message: deletion_error_summary(&errors),
                });
            }
        });
        cancellation_handle(cancellable)
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let entries = match request.source {
                RestoreSource::TrashEntries(entries) => entries
                    .into_iter()
                    .map(|entry| RestoreEntry {
                        source: entry.location,
                        display_name: entry.display_name,
                        original_target: None,
                        trash_info: None,
                    })
                    .collect(),
                RestoreSource::OriginalLocations(locations) => {
                    match trashed_entries_for_originals(&locations).await {
                        Ok(entries) => entries,
                        Err(error) => {
                            emit(OperationEvent::Failed {
                                request_id: request.id,
                                message: format!("Unable to find items in Trash: {error}"),
                            });
                            return;
                        }
                    }
                }
            };
            let total = entries.len();
            let mut errors = Vec::new();
            let mut restored_locations = Vec::new();
            let mut failed_locations = Vec::new();
            let mut affected_locations = HashSet::from([Location::uri("trash:///")]);
            for (index, entry) in entries.iter().enumerate() {
                if operation_cancellable.is_cancelled() {
                    emit(cancelled_event(
                        request.id,
                        restored_locations,
                        failed_locations,
                        entries[index..]
                            .iter()
                            .map(|entry| entry.source.clone())
                            .collect(),
                        affected_locations,
                    ));
                    return;
                }
                let source = gio_file(&entry.source);
                let result = if let Some(original_target) = entry.original_target.clone() {
                    let target = gio_file(&original_target);
                    if let Some(parent) = original_target.parent() {
                        affected_locations.insert(parent);
                    }
                    move_local(source, target, operation_cancellable.clone()).await
                } else {
                    match await_cancellable(
                        &source,
                        &operation_cancellable,
                        |source, cancellable, result| {
                            source.query_info_async(
                                "trash::orig-path",
                                gio::FileQueryInfoFlags::NONE,
                                glib::Priority::DEFAULT,
                                Some(cancellable),
                                move |output| result.resolve(output),
                            );
                        },
                    )
                    .await
                    {
                        Ok(info) => match info.attribute_byte_string("trash::orig-path") {
                            Some(original_path) => {
                                let target = gio::File::for_path(std::path::Path::new(
                                    original_path.as_str(),
                                ));
                                if let Some(parent) = location_for_file(&target)
                                    .and_then(|location| location.parent())
                                {
                                    affected_locations.insert(parent);
                                }
                                move_local(source, target, operation_cancellable.clone()).await
                            }
                            None => Err(glib::Error::new(
                                gio::IOErrorEnum::NotFound,
                                "The original location is unavailable",
                            )),
                        },
                        Err(error) => Err(error),
                    }
                };
                let restored_location = if let Err(error) = result {
                    if was_cancelled(&error) {
                        failed_locations.push(entry.source.clone());
                        emit(cancelled_event(
                            request.id,
                            restored_locations,
                            failed_locations,
                            entries[index + 1..]
                                .iter()
                                .map(|entry| entry.source.clone())
                                .collect(),
                            affected_locations,
                        ));
                        return;
                    }
                    errors.push(format!("{}: {error}", entry.display_name));
                    failed_locations.push(entry.source.clone());
                    None
                } else {
                    if let Some(info_path) = &entry.trash_info
                        && let Err(error) = std::fs::remove_file(info_path)
                    {
                        tracing::warn!(%error, "unable to remove restored trash metadata");
                    }
                    restored_locations.push(entry.source.clone());
                    Some(entry.source.clone())
                };
                emit(OperationEvent::RestoreProgress {
                    request_id: request.id,
                    completed: index + 1,
                    total,
                    restored_location,
                });
            }
            if errors.is_empty() {
                emit(OperationEvent::Restored {
                    request_id: request.id,
                    locations: restored_locations,
                });
            } else {
                emit(OperationEvent::RestoreCompletedWithErrors {
                    request_id: request.id,
                    restored_locations,
                    message: operation_error_summary(&errors, "restored"),
                });
            }
        });
        cancellation_handle(cancellable)
    }

    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancelled = Arc::new(AtomicBool::new(false));
        let task_cancelled = cancelled.clone();
        let task = glib::MainContext::default().spawn_local(async move {
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
            let result =
                write_staged_archive(&dest_dir, &archive_path, request.conflict, move |file| {
                    let count = count_files(&entries);
                    work_total.store(count, Ordering::Relaxed);
                    match format {
                        ArchiveFormat::Zip => {
                            compress_zip(file, &entries, password.as_deref(), &work_progress)
                        }
                        ArchiveFormat::SevenZ => {
                            compress_7z(file, &entries, password.as_deref(), &work_progress)
                        }
                        ArchiveFormat::TarGz => compress_tar(file, &entries, true, &work_progress),
                        ArchiveFormat::Tar => compress_tar(file, &entries, false, &work_progress),
                    }
                })
                .await;
            timer_id.remove();
            match result {
                Ok(()) => emit(OperationEvent::Compressed {
                    request_id: request.id,
                    archive_name: archive_name.clone(),
                }),
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error,
                }),
            }
        });
        LoadHandle::new(move || {
            cancelled.store(true, Ordering::Relaxed);
            task.abort();
        })
    }

    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancelled = Arc::new(AtomicBool::new(false));
        let task_cancelled = cancelled.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let Some(archive_path) = request.entry.location.native_path().map(Path::to_path_buf)
            else {
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
                    )
                }
                Some(ArchiveFormat::SevenZ) => {
                    let pw = password
                        .as_deref()
                        .map(sevenz_rust2::Password::from)
                        .unwrap_or_default();
                    let file = std::fs::File::open(&archive_path).map_err(|e| e.to_string())?;
                    extract_7z_from_reader(file, &dest_dir, pw, &work_progress)
                }
                Some(ArchiveFormat::TarGz) => {
                    extract_tar(&archive_path, &dest_dir, true, &work_progress)
                }
                Some(ArchiveFormat::Tar) => {
                    extract_tar(&archive_path, &dest_dir, false, &work_progress)
                }
                None => Err(format!("Unsupported archive format: {}", display_name)),
            })
            .await;
            timer_id.remove();
            match result {
                Ok(Ok(first_name)) => emit(OperationEvent::Extracted {
                    request_id: request.id,
                    first_name,
                }),
                Ok(Err(error)) => emit(OperationEvent::Failed {
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
            task.abort();
        })
    }
}

fn compress_zip(
    file: std::fs::File,
    entries: &[std::path::PathBuf],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
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
    for entry in entries {
        let name = entry
            .file_name()
            .ok_or("Entry has no file name")?
            .to_string_lossy()
            .to_string();
        if entry.is_dir() {
            add_dir_to_zip(&mut writer, entry, &name, &deflated, &stored, progress)?;
        } else {
            let opts = if is_incompressible(entry) {
                &stored
            } else {
                &deflated
            };
            writer.start_file(&name, *opts).map_err(|e| e.to_string())?;
            let f = std::fs::File::open(entry).map_err(|e| e.to_string())?;
            let f = std::io::BufReader::with_capacity(COPY_BUF, f);
            copy_with_big_buf(f, &mut writer).map_err(|e| e.to_string())?;
            progress.fetch_add(1, Ordering::Relaxed);
        }
    }
    writer
        .finish()
        .map_err(|error| error.to_string())?
        .into_inner()
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn add_dir_to_zip<W: std::io::Write + std::io::Seek>(
    writer: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    deflated: &zip::write::FileOptions<'_, ()>,
    stored: &zip::write::FileOptions<'_, ()>,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel_name = format!("{}/{}", prefix, entry.file_name().to_string_lossy());
        if path.is_dir() {
            add_dir_to_zip(writer, &path, &rel_name, deflated, stored, progress)?;
        } else {
            let opts = if is_incompressible(&path) {
                stored
            } else {
                deflated
            };
            writer
                .start_file(&rel_name, *opts)
                .map_err(|e| e.to_string())?;
            let f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            let f = std::io::BufReader::with_capacity(COPY_BUF, f);
            copy_with_big_buf(f, writer).map_err(|e| e.to_string())?;
            progress.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

fn compress_tar(
    file: std::fs::File,
    entries: &[std::path::PathBuf],
    gzip: bool,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    let writer = std::io::BufWriter::with_capacity(COPY_BUF, file);
    if gzip {
        let mut encoder = flate2::write::GzEncoder::new(writer, flate2::Compression::default());
        append_tar_entries(&mut encoder, entries, progress)?;
        encoder
            .finish()
            .map_err(|error| error.to_string())?
            .into_inner()
            .map_err(|error| error.to_string())?;
    } else {
        let mut writer = writer;
        append_tar_entries(&mut writer, entries, progress)?;
        writer.into_inner().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn append_tar_entries(
    writer: &mut dyn std::io::Write,
    entries: &[std::path::PathBuf],
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    let mut builder = tar::Builder::new(writer);
    for entry in entries {
        let name = entry
            .file_name()
            .ok_or("Entry has no file name")?
            .to_string_lossy()
            .to_string();
        if entry.is_dir() {
            builder
                .append_dir_all(&name, entry)
                .map_err(|e| e.to_string())?;
        } else {
            builder
                .append_path_with_name(entry, &name)
                .map_err(|e| e.to_string())?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
    builder.finish().map_err(|e| e.to_string())?;
    Ok(())
}

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

struct ExtractionDestination {
    root: OwnedFd,
}

impl ExtractionDestination {
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

    fn create_file(&self, path: &Path) -> Result<std::fs::File, String> {
        let parent = self.create_directories(path.parent().unwrap_or_else(|| Path::new("")))?;
        let name = path
            .file_name()
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        let name = self.available_name(&parent, name)?;
        rustix::fs::openat(
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
        .map_err(|error| error.to_string())
    }
}

/// Counts all files (not directories) under the given paths recursively.
fn count_files(entries: &[std::path::PathBuf]) -> usize {
    let mut count = 0;
    for entry in entries {
        if entry.is_dir() {
            if let Ok(rd) = std::fs::read_dir(entry) {
                for child in rd.flatten() {
                    count += count_files(&[child.path()]);
                }
            }
        } else {
            count += 1;
        }
    }
    count
}

const COPY_BUF: usize = 1 << 20; // 1 MiB

fn copy_with_big_buf(
    mut reader: impl std::io::Read,
    writer: &mut (impl std::io::Write + ?Sized),
) -> std::io::Result<u64> {
    let mut buf = vec![0u8; COPY_BUF];
    let mut total = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
    Ok(total)
}

/// Spawns a 100ms timer that polls progress counters and emits ArchiveProgress events.
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
        if timer_cancelled.load(Ordering::Relaxed) {
            return glib::ControlFlow::Break;
        }
        timer_emit(OperationEvent::ArchiveProgress {
            request_id,
            completed: timer_progress.load(Ordering::Relaxed),
            total: timer_total.load(Ordering::Relaxed),
        });
        glib::ControlFlow::Continue
    })
}

/// File extensions that are already compressed — storing them raw saves CPU with zero size gain.
const INCOMPRESSIBLE_EXTS: &[&str] = &[
    "zip", "7z", "gz", "bz2", "xz", "zst", "tar", "rar", "lz", "lz4", "br", "mp4", "mkv", "avi",
    "mov", "webm", "flv", "wmv", "jpg", "jpeg", "png", "webp", "gif", "heic", "avif", "bmp", "mp3",
    "flac", "aac", "ogg", "opus", "wma", "m4a", "pdf", "epub", "docx", "xlsx", "pptx", "odt",
    "ods", "odp", "iso", "dmg", "deb", "rpm", "apk", "jar", "war",
];

fn is_incompressible(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| INCOMPRESSIBLE_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Tracks renamed top-level entries so child paths follow the rename.
struct ExtractNameResolver {
    renames: std::collections::HashMap<OsString, OsString>,
}

impl ExtractNameResolver {
    fn new() -> Self {
        Self {
            renames: std::collections::HashMap::new(),
        }
    }

    /// Resolves a validated relative entry path to a conflict-free relative path.
    /// If the top-level component already exists, it's renamed to "name (2)", etc.
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

fn extract_zip_from_archive(
    archive: &mut zip::ZipArchive<std::fs::File>,
    dest_dir: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
) -> Result<Option<String>, String> {
    let destination = ExtractionDestination::open(dest_dir)?;
    let pw_bytes = password.map(|p| p.as_bytes());
    let mut resolver = ExtractNameResolver::new();
    let mut first_name = None;
    for i in 0..archive.len() {
        let read_options = zip::read::ZipReadOptions::new().password(pw_bytes);
        let mut entry = archive
            .by_index_with_options(i, read_options)
            .map_err(|e| e.to_string())?;
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
            let mut outfile = destination.create_file(&outpath)?;
            copy_with_big_buf(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(first_name)
}

fn extract_tar(
    archive_path: &Path,
    dest_dir: &Path,
    gzip: bool,
    progress: &Arc<AtomicUsize>,
) -> Result<Option<String>, String> {
    let destination = ExtractionDestination::open(dest_dir)?;
    let file = std::fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let reader: Box<dyn std::io::Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    let mut resolver = ExtractNameResolver::new();
    let mut first_name = None;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let name = entry.path().map_err(|e| e.to_string())?;
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
            let mut outfile = destination.create_file(&outpath)?;
            copy_with_big_buf(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(first_name)
}

fn compress_7z(
    file: std::fs::File,
    entries: &[std::path::PathBuf],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
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
    for entry in entries {
        add_path_to_7z(&mut writer, entry, entry, progress)?;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn add_path_to_7z(
    writer: &mut sevenz_rust2::ArchiveWriter<std::fs::File>,
    base: &Path,
    path: &Path,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    if path.is_dir() {
        for child in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            add_path_to_7z(writer, base, &child.path(), progress)?;
        }
    } else {
        let name = path
            .strip_prefix(base.parent().unwrap_or(base))
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let entry = sevenz_rust2::ArchiveEntry::from_path(path, name);
        let reader = std::fs::File::open(path).map_err(|e| e.to_string())?;
        writer
            .push_archive_entry(entry, Some(reader))
            .map_err(|e| e.to_string())?;
        progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

fn extract_7z_from_reader(
    reader: std::fs::File,
    dest_dir: &Path,
    password: sevenz_rust2::Password,
    progress: &Arc<AtomicUsize>,
) -> Result<Option<String>, String> {
    let destination = ExtractionDestination::open(dest_dir)?;
    let resolver = std::cell::RefCell::new(ExtractNameResolver::new());
    let first_name = std::cell::RefCell::new(None::<String>);
    let progress = progress.clone();
    sevenz_rust2::decompress_with_extract_fn_and_password(
        reader,
        dest_dir,
        password,
        |entry, reader, _safe_path| {
            let path = validated_archive_path(&entry.name)
                .map_err(|e| sevenz_rust2::Error::Other(e.into()))?;
            let outpath = resolver
                .borrow_mut()
                .resolve(&destination, &path)
                .map_err(|e| sevenz_rust2::Error::Other(e.into()))?;
            if first_name.borrow().is_none() {
                *first_name.borrow_mut() = outpath
                    .components()
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().to_string());
            }
            if entry.is_directory {
                destination
                    .create_directories(&outpath)
                    .map_err(|e| sevenz_rust2::Error::Other(e.into()))?;
            } else {
                let mut file = destination
                    .create_file(&outpath)
                    .map_err(|e| sevenz_rust2::Error::Other(e.into()))?;
                copy_with_big_buf(reader, &mut file)
                    .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
            }
            progress.fetch_add(1, Ordering::Relaxed);
            Ok(false)
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(first_name.into_inner())
}
