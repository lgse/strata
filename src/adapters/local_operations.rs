// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

mod archive;
mod case_variants;
mod names;
mod native_fs;
use case_variants::CaseVariants;
use names::{Key, PlanEntry, Planned, Taken, fix::fix_name, numbered_names, plan_names};
#[cfg(test)]
use native_fs::LocalFileIdentity;
use native_fs::Recorded;
pub(super) use native_fs::open_local_child_directory;
#[cfg(test)]
use native_fs::retry_local_open;

use native_fs::{open_local_parent_directory, os_error_text};

mod progress;
use progress::BYTE_PROGRESS_MIN_FILE_SIZE;
use progress::FileTransferProgress;
use progress::Reported;
#[cfg(test)]
use progress::TRANSFER_PROGRESS_INTERVAL;
use progress::TransferProgressTracker;
use progress::copy_file_with_progress;
use progress::move_file_with_progress;
mod replacement;
#[cfg(test)]
use replacement::CopyStep;
#[cfg(test)]
use replacement::MergeCopied;
#[cfg(test)]
use replacement::MergeCopy;
#[cfg(test)]
use replacement::StageContent;
#[cfg(test)]
use replacement::StagedSibling;
use replacement::placement_error;
#[cfg(test)]
use replacement::publish_staged_without_replace;

use replacement::split_local_path;

use replacement::Copying;
#[cfg(test)]
use replacement::MergeHooks;
#[cfg(test)]
use replacement::MergePlan;
#[cfg(test)]
use replacement::StageOverwrite;
use replacement::can_trash;
#[cfg(test)]
use replacement::merge_copy;
#[cfg(test)]
use replacement::merge_local;
#[cfg(test)]
use replacement::merge_local_with;
use replacement::merge_owing;
use replacement::progress_merge_hooks;
#[cfg(test)]
use replacement::publish_staged_replacement;
#[cfg(test)]
use replacement::replace_local;
#[cfg(test)]
use replacement::replace_local_with;
use replacement::replace_owing;
#[cfg(test)]
use replacement::trash_stage_overwrite;
mod deletion;
#[cfg(test)]
use deletion::LocalDeleteJob;
#[cfg(test)]
use deletion::LocalDeleteQueue;
#[cfg(test)]
use deletion::LocalDeleteStep;
#[cfg(test)]
use deletion::PinnedEntry;
#[cfg(test)]
use deletion::RemovedReporter;
use deletion::bounded_local_worker_count;
#[cfg(test)]
use deletion::ensure_local_delete_target_unchanged;
#[cfg(test)]
use deletion::local_delete_worker_count;
use deletion::local_file_identity;
#[cfg(test)]
use deletion::open_local_delete_target;
#[cfg(test)]
use deletion::parallel_delete_local;
#[cfg(test)]
use deletion::parallel_delete_local_blocking_with_workers;
#[cfg(test)]
use deletion::parallel_delete_local_with;
use deletion::permanently_delete;
#[cfg(test)]
use deletion::permanently_delete_local;
#[cfg(test)]
use deletion::permanently_delete_local_path_if_unchanged;
#[cfg(test)]
use deletion::process_local_delete_job;
#[cfg(test)]
use deletion::run_local_delete_step;
#[cfg(test)]
use deletion::run_local_delete_workers;

#[cfg(test)]
use deletion::run_deletion_in;
use deletion::{DeletionTarget, permanently_delete_maybe_local_if_unchanged, run_deletion};
mod restore;
use restore::RestoreEntry;
#[cfg(test)]
use restore::StagedOriginalLookup;
#[cfg(test)]
use restore::move_restore_path;
use restore::restore_renamed;
use restore::restore_trash_entry;
use restore::restore_worker_count;
use restore::restored_destination;
use restore::run_merge_undo;
#[cfg(test)]
use restore::trash_entries_at;
use restore::trashed_entries_for_originals;
use restore::trashed_merge_original;
#[cfg(test)]
use restore::{NameKept, restore_name};

mod copy_engine;
mod create_entry;
mod deferred_close;
use deferred_close::WaitCancelled;
mod environment;
mod fs_ops;
mod holders;
mod leftovers;
pub(super) use leftovers::Deadline;
mod merge_aside;
mod mount_profile;
mod owed;
mod placement;
pub(crate) mod probes;
mod release;
pub(crate) use release::ActiveOperation;
mod transfer_targets;
pub(super) mod trash;
mod typed_names;
mod work_queue;

#[cfg(test)]
pub(crate) use archive::ArchiveListing;
pub(crate) use archive::{
    ARCHIVE_PREVIEW_FAILED_MESSAGE, ARCHIVE_TOO_LARGE_MESSAGE, ARCHIVE_UNSUPPORTED_MESSAGE,
    ArchiveListingStatus, INVALID_ARCHIVE, MAX_ARCHIVE_PASSWORD_BYTES, MAYBE_BAD_PASSWORD,
    PASSWORD_REQUIRED, archive_payload_valid, decode_archive_listing, encode_archive_result,
    list_archive_entries_direct,
};

#[cfg(test)]
pub(crate) use archive::write_compression_fixture;
pub(crate) use fs_ops::Watched;
pub(in crate::adapters) use fs_ops::{
    FolderStamp, folder_stamp, list_folder, stamp_mount, unwatch, watch,
};
#[cfg(test)]
pub(crate) use tests::fs_quirks::{Births, FsQuirks, Op as FsOp, Quirks, evict};
pub(crate) use transfer_targets::transfer_targets;
pub(crate) use typed_names::{
    refused_in as typed_name_refusal, rename_taken as typed_rename_is_taken,
};

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    future::Future,
    io,
    os::{fd::AsFd, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use gtk::{gio, glib, prelude::*};

use fs_ops::{Entry, fs};
use mount_profile::{FolderNames, NamePolicy};
use owed::{Deletion, Done, Owed, Settling};
use placement::{PlaceError, RenameFailure};

use crate::{
    adapters::{
        gio_close, gio_file_for_location, location_for_file,
        trash_restore::RestoreContext,
        trash_support::{TrashError, TrashFailure, trash_with_cleanup},
        volume::{NameProfile, mount_watch},
    },
    model::{Location, hidden_names::HiddenName},
    services::{
        CancelledOperation, CompressRequest, CreateDirectoryRequest, CreateFileRequest,
        DeleteRequest, ExtractRequest, KeptCause, KeptItem, LoadHandle, NameMode, OperationEvent,
        OperationProvider, OperationRequestId, PasteItem, PasteRequest, RecoverRequest,
        RenameCause, RenameRequest, RenamedItem, ReplacedOriginal, RestoreRequest, RestoreSource,
        SkippedItem, SpecialKind, TransferConflict, TransferReport, UndoCopyRequest,
        UndoMergeRequest, UndoMoveRequest, UndoRenameRequest, validate_basename,
    },
};

const INVALID_NAME_REASON: &str = crate::services::INVALID_NAME_REASON;
const UNCONFIRMED_REASON: &str =
    "The result could not be confirmed. Check both locations before trying again.";
const UNSUPPORTED_REASON: &str =
    "This location can\u{2019}t guarantee that another item won\u{2019}t be overwritten";

#[derive(Clone, Copy, Debug, Eq, PartialEq, glib::ErrorDomain)]
#[error_domain(name = "strata-unheld")]
enum Unheld {
    Symlink,
}

fn unheld(error: &glib::Error) -> Option<SpecialKind> {
    error.kind::<Unheld>().map(|kind| match kind {
        Unheld::Symlink => SpecialKind::Symlink,
    })
}

fn left_out(
    source: &Location,
    destination: &gio::File,
    kind: SpecialKind,
    moving: bool,
) -> TransferReport {
    if moving {
        return TransferReport {
            kept: vec![KeptItem {
                location: source.clone(),
                cause: KeptCause::Skipped(kind),
            }],
            ..TransferReport::default()
        };
    }
    let target = source.file_name().map(|name| destination.child(name));
    TransferReport {
        skipped: target
            .and_then(|target| location_for_file(&target))
            .map(|location| SkippedItem { location, kind })
            .into_iter()
            .collect(),
        ..TransferReport::default()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MountFlushHint {
    pub root: PathBuf,
    pub removable: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum SyncDestination<'a> {
    Local(&'a Path),
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "copy helper tests pass a destination that has no local path"
        )
    )]
    NonLocal,
}

fn blocking_join_message(error: Box<dyn std::any::Any + Send>) -> String {
    error
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            error
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "filesystem sync panicked".to_owned())
}

fn device_flush_failure(error: &str) -> String {
    rust_i18n::t!(
        "The device could not finish writing: %{error}. Earlier writes may still be pending; wait for safe eject before unplugging.",
        error = error
    )
    .into_owned()
}

pub(crate) fn sync_filesystem(root: &Path) -> io::Result<()> {
    let handle = fs()
        .open_path(
            root.as_os_str(),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(io::Error::from)?;
    fs().sync_fs(std::os::fd::AsFd::as_fd(&handle))
        .map_err(io::Error::from)?;
    Ok(())
}

pub(crate) fn flush_filesystem(root: &Path) -> io::Result<()> {
    #[cfg(test)]
    if let Some(kind) = sync_probe_before(root) {
        return Err(io::Error::new(kind, "flush failed"));
    }
    sync_filesystem(root)
}

#[cfg(test)]
mod sync_probe {
    use std::{
        io,
        path::{Path, PathBuf},
        sync::{Arc, Condvar, Mutex, atomic::AtomicBool, atomic::Ordering},
        thread,
    };

    struct SyncGate {
        started: AtomicBool,
        released: Mutex<bool>,
        released_cv: Condvar,
    }

    struct SyncProbeState {
        fail: Option<io::ErrorKind>,
        gate: Option<Arc<SyncGate>>,
        calls: Vec<SyncObservation>,
    }

    type SyncObserver = Arc<dyn Fn(&Path) + Send + Sync>;

    static SYNC_PROBE: Mutex<Option<SyncProbeState>> = Mutex::new(None);
    static SYNC_OBSERVER: Mutex<Option<SyncObserver>> = Mutex::new(None);

    #[derive(Clone, Debug)]
    pub(crate) struct SyncObservation {
        pub root: PathBuf,
        pub thread_id: thread::ThreadId,
    }

    pub(crate) fn install_sync_probe(fail: Option<io::ErrorKind>, gate: bool) {
        let gate = gate.then(|| {
            Arc::new(SyncGate {
                started: AtomicBool::new(false),
                released: Mutex::new(false),
                released_cv: Condvar::new(),
            })
        });
        let mut slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        *slot = Some(SyncProbeState {
            fail,
            gate,
            calls: Vec::new(),
        });
    }

    pub(crate) fn clear_sync_probe() {
        release_sync_probe();
        let mut slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        *slot = None;
    }

    pub(crate) fn set_sync_observer(observer: Option<SyncObserver>) {
        let mut slot = SYNC_OBSERVER
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *slot = observer;
    }

    pub(crate) fn release_sync_probe() {
        let gate = {
            let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
            slot.as_ref().and_then(|probe| probe.gate.clone())
        };
        let Some(gate) = gate else {
            return;
        };
        let mut released = gate
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *released = true;
        gate.released_cv.notify_all();
    }

    pub(crate) fn sync_probe_is_blocked() -> bool {
        let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        slot.as_ref()
            .and_then(|probe| probe.gate.as_ref())
            .is_some_and(|gate| gate.started.load(Ordering::SeqCst))
    }

    pub(crate) fn sync_probe_len() -> usize {
        let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        slot.as_ref().map(|probe| probe.calls.len()).unwrap_or(0)
    }

    pub(crate) fn sync_probe_observations() -> Vec<SyncObservation> {
        let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        slot.as_ref()
            .map(|probe| probe.calls.clone())
            .unwrap_or_default()
    }

    pub(super) fn sync_probe_before(root: &Path) -> Option<io::ErrorKind> {
        let (fail, gate) = {
            let mut slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
            let probe = slot.as_mut()?;
            probe.calls.push(SyncObservation {
                root: root.to_path_buf(),
                thread_id: thread::current().id(),
            });
            (probe.fail, probe.gate.clone())
        };
        let observer = SYNC_OBSERVER
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(observer) = observer {
            observer(root);
        }
        if let Some(gate) = gate {
            gate.started.store(true, Ordering::SeqCst);
            let mut released = gate
                .released
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while !*released {
                released = gate
                    .released_cv
                    .wait(released)
                    .unwrap_or_else(|error| error.into_inner());
            }
        }
        fail
    }
}

#[cfg(test)]
use sync_probe::sync_probe_before;
#[cfg(test)]
pub(crate) use sync_probe::{
    clear_sync_probe, install_sync_probe, release_sync_probe, set_sync_observer,
    sync_probe_is_blocked, sync_probe_len, sync_probe_observations,
};

#[cfg(test)]
thread_local! {
    static REMOVABLE_ROOTS_FOR_TEST: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_removable_roots_for_test(roots: Option<Vec<PathBuf>>) {
    REMOVABLE_ROOTS_FOR_TEST.with(|slot| *slot.borrow_mut() = roots);
}

#[cfg(test)]
fn removable_hints_for_test() -> Option<Vec<MountFlushHint>> {
    REMOVABLE_ROOTS_FOR_TEST.with(|slot| {
        slot.borrow().as_ref().map(|roots| {
            roots
                .iter()
                .map(|root| MountFlushHint {
                    root: root.clone(),
                    removable: true,
                })
                .collect()
        })
    })
}

pub(crate) fn removable_sync_roots(
    destinations: &[SyncDestination<'_>],
    hints: &[MountFlushHint],
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for destination in destinations {
        let SyncDestination::Local(path) = destination else {
            continue;
        };
        let Some(hint) = hints
            .iter()
            .filter(|hint| path.starts_with(&hint.root))
            .max_by_key(|hint| hint.root.as_os_str().len())
        else {
            continue;
        };
        if hint.removable && !roots.iter().any(|root| root == &hint.root) {
            roots.push(hint.root.clone());
        }
    }
    roots
}

/// Reads the mount table off the main thread; the volume monitor is
/// in-memory GIO state and is matched here.
async fn removable_roots_for_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        return Vec::new();
    }
    let destinations: Vec<_> = paths
        .iter()
        .map(|path| SyncDestination::Local(path))
        .collect();
    #[cfg(test)]
    if let Some(hints) = removable_hints_for_test() {
        return removable_sync_roots(&destinations, &hints);
    }
    let table = mount_watch::fresh().await;
    let mounts = gio::VolumeMonitor::get().mounts();
    let mut hints = Vec::new();
    for path in paths {
        let Some(root) = table.mount_point_for(path).map(Path::to_path_buf) else {
            continue;
        };
        if hints.iter().any(|hint: &MountFlushHint| hint.root == root) {
            continue;
        }
        let removable = mounts.iter().any(|mount| {
            mount.root().path().as_deref() == Some(root.as_path())
                && mount_drive_is_removable(mount)
        });
        hints.push(MountFlushHint { root, removable });
    }
    removable_sync_roots(&destinations, &hints)
}

fn mount_drive_is_removable(mount: &gio::Mount) -> bool {
    let drive = mount
        .drive()
        .or_else(|| mount.volume().and_then(|volume| volume.drive()));
    drive.is_some_and(|drive| drive.is_removable() || drive.is_media_removable())
}

async fn flush_removable_writes(
    paths: Vec<PathBuf>,
    cancellable: &gio::Cancellable,
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
    completed: &[Location],
    affected_locations: HashSet<Location>,
) -> bool {
    let roots = removable_roots_for_paths(&paths).await;
    if roots.is_empty() {
        if cancellable.is_cancelled() {
            emit(cancelled_event(
                request_id,
                completed.to_vec(),
                Vec::new(),
                Vec::new(),
                affected_locations,
            ));
            return false;
        }
        return true;
    }
    emit(OperationEvent::FlushingToDevice { request_id });
    for root in roots {
        let synced = gio::spawn_blocking(move || flush_filesystem(&root)).await;
        let error = match synced {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => os_error_text(error),
            Err(error) => blocking_join_message(error),
        };
        emit(OperationEvent::TransferFailed {
            request_id,
            completed_locations: completed.to_vec(),
            message: device_flush_failure(&error),
        });
        return false;
    }
    if cancellable.is_cancelled() {
        emit(cancelled_event(
            request_id,
            completed.to_vec(),
            Vec::new(),
            Vec::new(),
            affected_locations,
        ));
        return false;
    }
    true
}

async fn flush_written_roots(
    paths: &[PathBuf],
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
) -> Result<(), String> {
    let roots = removable_roots_for_paths(paths).await;
    if roots.is_empty() {
        return Ok(());
    }
    emit(OperationEvent::FlushingToDevice { request_id });
    for root in roots {
        let result = gio::spawn_blocking(move || flush_filesystem(&root)).await;
        let error = match result {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => os_error_text(error),
            Err(error) => blocking_join_message(error),
        };
        return Err(device_flush_failure(&error));
    }
    Ok(())
}

struct TransferStop {
    completed: Vec<Location>,
    failed: Vec<Location>,
    not_attempted: Vec<Location>,
    affected_locations: HashSet<Location>,
    failure: Option<String>,
    skip_flush: bool,
}

fn joined_failures(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(crate::i18n::join_sentences(&first, &second)),
        (first, second) => first.or(second),
    }
}

async fn stop_transfer(
    paths: &[PathBuf],
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
    stop: TransferStop,
) {
    let flush_error = if stop.skip_flush {
        None
    } else {
        flush_written_roots(paths, emit, request_id).await.err()
    };
    match (stop.failure, flush_error) {
        (failure, Some(flush_error)) => emit(OperationEvent::TransferFailed {
            request_id,
            completed_locations: stop.completed,
            message: failure.map_or(flush_error.clone(), |message| {
                crate::i18n::join_sentences(&message, &flush_error)
            }),
        }),
        (Some(message), None) => emit(OperationEvent::TransferFailed {
            request_id,
            completed_locations: stop.completed,
            message,
        }),
        (None, None) => emit(cancelled_event(
            request_id,
            stop.completed,
            stop.failed,
            stop.not_attempted,
            stop.affected_locations,
        )),
    }
}

pub(super) async fn await_cancellable<O, T>(
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

/// GLib calls a backend's rename without checking it has one, and GDummyFile, which serves
/// `smb://` without a running GVfs, has none.
async fn set_display_name(
    file: &gio::File,
    name: &str,
    cancellable: &gio::Cancellable,
) -> Result<gio::File, glib::Error> {
    let renames = file.interface::<gio::File>().is_some_and(|interface| {
        let interface: &gio::ffi::GFileIface = (*interface).as_ref();
        interface.set_display_name.is_some()
    });
    if !renames {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::services::backend_unavailable_message(&file.uri()),
        ));
    }
    let name = name.to_owned();
    await_cancellable(file, cancellable, move |file, cancellable, result| {
        file.set_display_name_async(
            &name,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
}

async fn join_local_tasks<K, T>(
    tasks: Vec<(K, glib::JoinHandle<T>)>,
) -> Vec<(K, Result<T, glib::JoinError>)>
where
    T: 'static,
{
    let mut results = Vec::with_capacity(tasks.len());
    for (key, task) in tasks {
        results.push((key, task.await));
    }
    results
}

async fn run_cancellable<T: Send + 'static>(
    cancellable: &gio::Cancellable,
    work: impl FnOnce(Arc<AtomicBool>) -> T + Send + 'static,
) -> Result<T, Box<dyn std::any::Any + Send>> {
    let cancelled = Arc::new(AtomicBool::new(cancellable.is_cancelled()));
    let flag = cancelled.clone();
    let handler = cancellable.connect_cancelled(move |_| flag.store(true, Ordering::Release));
    let result = gio::spawn_blocking(move || work(cancelled)).await;
    if let Some(handler) = handler {
        cancellable.disconnect_cancelled(handler);
    }
    result
}

const MAX_LOCAL_IO_WORKERS: usize = 8;

fn copy_failure_on_fat32(error: &glib::Error, fat32_destination: bool) -> (Option<String>, bool) {
    if was_cancelled(error) {
        return (None, false);
    }
    if !fat32_destination {
        return (Some(crate::services::gio_error_message(error)), false);
    }
    let description = if error.message().contains("File too large") {
        crate::i18n::tr(
            "A file is too large for this FAT32 drive (maximum file size: 4 GiB). Use an exFAT or another large-file-capable drive instead.",
        )
    } else {
        crate::services::gio_error_message(error)
    };
    let details = rust_i18n::t!(
        "Earlier completed copies may still be writing; wait for safe eject before unplugging. Details: %{error}",
        error = crate::services::gio_error_detail(error)
    );
    (
        Some(crate::i18n::join_sentences(&description, &details)),
        true,
    )
}

/// Size probes never fail with NoSpace on their own, so the code marks this refusal.
fn fat32_file_too_large(name: &str) -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::NoSpace,
        &rust_i18n::t!(
            "%{name} is too large for a FAT32 drive (maximum file size: 4 GiB). Use an exFAT or another large-file-capable drive instead.",
            name = name
        ),
    )
}

#[derive(Clone)]
struct TransferEstimate {
    bytes: Option<u64>,
    files: usize,
    kind: gio::FileType,
    filesystem: Option<glib::GString>,
}

fn transfer_size(
    file: gio::File,
    cancellable: gio::Cancellable,
    max_file_size: Option<u64>,
) -> Pin<Box<dyn Future<Output = Result<TransferEstimate, glib::Error>>>> {
    Box::pin(async move {
        let info = await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type,standard::size,id::filesystem",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        let filesystem = info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM);
        if info.file_type() != gio::FileType::Directory {
            let size = info
                .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_SIZE)
                .then(|| info.size().max(0) as u64);
            if info.file_type() == gio::FileType::Regular
                && let (Some(limit), Some(size)) = (max_file_size, size)
                && size > limit
            {
                return Err(fat32_file_too_large(
                    &transfer_source_name(&file)
                        .unwrap_or_default()
                        .to_string_lossy(),
                ));
            }
            return Ok(TransferEstimate {
                bytes: size,
                files: 1,
                kind: info.file_type(),
                filesystem,
            });
        }

        let enumerator = gio_close::enumerate_children(
            &file,
            "standard::name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(&cancellable),
        )
        .await?;
        let mut total = Some(0_u64);
        let mut files = 0_usize;
        loop {
            let children = await_cancellable(
                &*enumerator,
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
                return Ok(TransferEstimate {
                    bytes: total,
                    files,
                    kind: gio::FileType::Directory,
                    filesystem,
                });
            }
            for chunk in children.chunks(MAX_LOCAL_IO_WORKERS) {
                let context = glib::MainContext::default();
                let tasks = chunk
                    .iter()
                    .map(|child| {
                        context.spawn_local(transfer_size(
                            file.child(child.name()),
                            cancellable.clone(),
                            max_file_size,
                        ))
                    })
                    .collect::<Vec<_>>();
                for task in tasks {
                    let child_size = task
                        .await
                        .map_err(|_| io_error("Transfer-size worker failed"))??;
                    total = total.and_then(|total| {
                        child_size.bytes.and_then(|size| total.checked_add(size))
                    });
                    files = files.saturating_add(child_size.files);
                }
            }
        }
    })
}

#[derive(Debug)]
struct TransferSizes {
    sizes: Vec<Option<u64>>,
    total: Option<u64>,
    files: Vec<Option<usize>>,
    total_files: Option<usize>,
    kinds: Vec<Option<gio::FileType>>,
    filesystems: Vec<Option<glib::GString>>,
}

async fn transfer_sizes(
    files: &[gio::File],
    cancellable: &gio::Cancellable,
    max_file_size: Option<u64>,
) -> Result<TransferSizes, glib::Error> {
    let mut sizes = Vec::with_capacity(files.len());
    let mut counts = Vec::with_capacity(files.len());
    let mut total = Some(0_u64);
    let mut total_files = Some(0_usize);
    let mut kinds = Vec::with_capacity(files.len());
    let mut filesystems = Vec::with_capacity(files.len());
    for chunk in files.chunks(MAX_LOCAL_IO_WORKERS) {
        let context = glib::MainContext::default();
        let tasks = chunk
            .iter()
            .map(|file| {
                context.spawn_local(transfer_size(
                    file.clone(),
                    cancellable.clone(),
                    max_file_size,
                ))
            })
            .collect::<Vec<_>>();
        for task in tasks {
            let size = match task.await {
                Ok(Ok(size)) => Some(size),
                Ok(Err(error))
                    if was_cancelled(&error) || error.matches(gio::IOErrorEnum::NoSpace) =>
                {
                    return Err(error);
                }
                Ok(Err(_)) | Err(_) => None,
            };
            total = total.and_then(|total| {
                size.as_ref()
                    .and_then(|size| size.bytes.and_then(|bytes| total.checked_add(bytes)))
            });
            total_files = total_files
                .and_then(|total| size.as_ref().map(|size| total.saturating_add(size.files)));
            counts.push(size.as_ref().map(|size| size.files));
            sizes.push(size.as_ref().and_then(|size| size.bytes));
            kinds.push(size.as_ref().map(|size| size.kind));
            filesystems.push(size.and_then(|size| size.filesystem));
        }
    }
    Ok(TransferSizes {
        sizes,
        total,
        files: counts,
        total_files,
        kinds,
        filesystems,
    })
}

fn validated_child(parent: &gio::File, name: &str) -> Result<gio::File, &'static str> {
    validate_basename(name)?;
    Ok(parent.child(name))
}

fn transfer_is_noop(source: &gio::File, destination: &gio::File, target: &gio::File) -> bool {
    source.equal(target)
        || source.equal(destination)
        || destination.has_prefix(source)
        || target_is_within_source(source, target) == Some(true)
}

/// Compares device/inode identities so symlink, bind-mount, and
/// case-insensitive aliases of the source are caught. A symlink source keeps
/// its own identity, so moving a link into the folder it targets is allowed.
/// `None` means an ancestor could not be inspected.
fn target_is_within_source(source: &gio::File, target: &gio::File) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;

    fs_ops::assert_off_main_thread();
    let (Some(source_path), Some(target_path)) = (source.path(), target.path()) else {
        return Some(false);
    };
    let identity = |metadata: &std::fs::Metadata| (metadata.dev(), metadata.ino());
    let source_identity = identity(&std::fs::symlink_metadata(source_path).ok()?);
    if std::fs::symlink_metadata(&target_path)
        .is_ok_and(|metadata| identity(&metadata) == source_identity)
    {
        return Some(true);
    }
    for ancestor in target_path.ancestors().skip(1) {
        match std::fs::metadata(ancestor) {
            Ok(metadata) if identity(&metadata) == source_identity => return Some(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    Some(false)
}

pub(crate) fn transfer_source_name(source: &gio::File) -> Option<OsString> {
    crate::model::transfer_file_name(source)
}

/// Items a copy pastes into their own folder. Each is numbered as it is copied, so it takes no
/// name in the plan and another item of its name meets the original.
fn duplicates(sources: &[gio::File], destination: &gio::File, moving: bool) -> Vec<bool> {
    sources
        .iter()
        .map(|source| {
            !moving
                && source
                    .parent()
                    .is_some_and(|parent| parent.equal(destination))
        })
        .collect()
}

/// Items whose names the plan gives are copied first, then duplicates, then the conflicts the
/// prompt resolved, so an item numbered as it is copied never takes a planned name and a duplicate
/// is copied before Replace can remove its original.
fn in_copy_order(mut request: PasteRequest, destination: &gio::File) -> PasteRequest {
    let sources = request
        .items
        .iter()
        .map(|item| gio_file_for_location(&item.source))
        .collect::<Vec<_>>();
    let duplicate = duplicates(&sources, destination, request.move_sources);
    let mut ranked = std::mem::take(&mut request.items)
        .into_iter()
        .zip(duplicate)
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(item, duplicate)| match (*duplicate, item.conflict) {
        (false, TransferConflict::FailIfExists) => 0,
        (true, _) => 1,
        (false, _) => 2,
    });
    request.items = ranked.into_iter().map(|(item, _)| item).collect();
    request
}

/// A `renamable` item is numbered where an earlier item has its name, and gets the destination's
/// rules where it is `crossing` too. A `duplicate` keeps its name.
fn paste_targets(
    names: &[Option<OsString>],
    rules: &NameProfile,
    (crossing, renamable, duplicate): (&[bool], &[bool], &[bool]),
    kinds: &[Option<gio::FileType>],
) -> Result<Vec<Option<Planned>>, PlaceError> {
    let entries = (0..names.len())
        .filter(|index| !duplicate[*index])
        .filter_map(|index| {
            Some(PlanEntry {
                name: names[index].as_deref()?,
                is_directory: kinds[index] == Some(gio::FileType::Directory),
                fixing: crossing[index] && renamable[index],
                repeatable: renamable[index],
            })
        })
        .collect::<Vec<_>>();
    let mut planned = plan_names(&entries, rules, Key::Folded)?.into_iter();
    Ok(names
        .iter()
        .zip(duplicate)
        .map(|(name, duplicate)| match (name, duplicate) {
            (Some(name), true) => Some(Planned {
                target: name.clone(),
                cause: None,
            }),
            (Some(_), false) => planned.next(),
            (None, _) => None,
        })
        .collect())
}

fn planned_paste(
    items: &[PasteItem],
    sources: &[gio::File],
    rules: &NameProfile,
    (crossing, duplicate): (&[bool], &[bool]),
    kinds: &[Option<gio::FileType>],
) -> Result<Vec<Option<Planned>>, PlaceError> {
    let own = sources.iter().map(transfer_source_name).collect::<Vec<_>>();
    let names = items
        .iter()
        .zip(&own)
        .map(|(item, own)| match &item.target_name {
            Some(shown) => plain_name(&shown.name).then(|| shown.name.clone()),
            None => own.clone(),
        })
        .collect::<Vec<_>>();
    // Replace and Merge act on the name the user saw, which is the item's
    // own where no prompt showed another.
    let renamable = items
        .iter()
        .map(|item| {
            item.target_name.is_none()
                && !matches!(
                    item.conflict,
                    TransferConflict::ReplaceExisting | TransferConflict::Merge
                )
        })
        .collect::<Vec<_>>();
    let mut planned = paste_targets(&names, rules, (crossing, &renamable, duplicate), kinds)?;
    for (index, item) in items.iter().enumerate() {
        let (Some(shown), Some(planned), Some(name)) =
            (&item.target_name, &mut planned[index], &own[index])
        else {
            continue;
        };
        let is_directory = kinds[index] == Some(gio::FileType::Directory);
        planned.cause = (*name != shown.name).then(|| {
            shown
                .cause
                .or_else(|| fix_name(name, rules, is_directory).cause)
                .unwrap_or(RenameCause::Alias)
        });
    }
    Ok(planned)
}

fn plain_name(name: &OsStr) -> bool {
    !matches!(name.as_bytes(), b"" | b"." | b"..")
        && !name
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'/' | b'\0'))
}

#[derive(Clone)]
struct ParallelCopyPlan {
    index: usize,
    source_path: PathBuf,
    target_location: Location,
    written_path: PathBuf,
    renamed: Option<RenameCause>,
}

async fn parallel_copy_plans(
    request: &PasteRequest,
    sources: &[gio::File],
    destination: &gio::File,
    planned: &[Option<Planned>],
    kinds: &[Option<gio::FileType>],
    windows_disk: bool,
) -> Option<(Vec<ParallelCopyPlan>, usize)> {
    if request.items.len() <= MAX_LOCAL_IO_WORKERS
        || !destination.is_native()
        || request
            .items
            .iter()
            .any(|item| item.conflict != TransferConflict::FailIfExists)
    {
        return None;
    }

    let targets = planned
        .iter()
        .zip(kinds)
        .map(|(planned, kind)| {
            planned
                .as_ref()
                .filter(|_| *kind == Some(gio::FileType::Regular))
                .map(|planned| (destination.child(&planned.target), planned.cause))
        })
        .collect::<Vec<_>>();
    let (sources, destination) = (sources.to_vec(), destination.clone());
    gio::spawn_blocking(move || {
        let plans = sources
            .iter()
            .zip(targets)
            .enumerate()
            .map(|(index, (source, target))| {
                let (target, renamed) = target.filter(|_| source.is_native())?;
                if transfer_is_noop(source, &destination, &target) {
                    return None;
                }
                Some(ParallelCopyPlan {
                    index,
                    source_path: source.path()?,
                    target_location: location_for_file(&target)?,
                    written_path: target.path()?,
                    renamed,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let available = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        // A rotational disk on either side reads or writes one file at a time;
        // a disk Windows formats is taken for one where its device does not say.
        let source_folders: HashSet<&Path> = plans
            .iter()
            .filter_map(|plan| plan.source_path.parent())
            .collect();
        let rotational = destination
            .path()
            .and_then(|path| std::fs::File::open(path).ok())
            .and_then(|directory| fs().rotational(directory.as_fd()))
            .unwrap_or(windows_disk)
            || source_folders.into_iter().any(|folder| {
                open_local_parent_directory(folder)
                    .ok()
                    .and_then(|folder| fs().rotational(folder.as_fd()))
                    == Some(true)
            });
        Some((plans, bounded_local_worker_count(available, rotational)))
    })
    .await
    .ok()
    .flatten()
}

struct ParallelCopyOutcome {
    failed: Vec<(usize, glib::Error)>,
    next: usize,
    written_paths: Vec<PathBuf>,
}

async fn execute_parallel_copy_plans(
    plans: &[ParallelCopyPlan],
    (item_sizes, item_files): (&[Option<u64>], &[Option<usize>]),
    (worker_count, moving): (usize, bool),
    cancellable: &gio::Cancellable,
    copying: &Copying,
    (settling, variants): (&mut Settling, &mut CaseVariants),
) -> ParallelCopyOutcome {
    let mut outcome = ParallelCopyOutcome {
        failed: Vec::new(),
        next: 0,
        written_paths: Vec::new(),
    };
    let last = plans.last().map(|plan| plan.index);
    while outcome.next < plans.len() && !cancellable.is_cancelled() {
        let start = outcome.next;
        let full = (start + worker_count).min(plans.len());
        // An item waits for the earlier ones whose names differ from its own only in case.
        let end = (start + 1..full)
            .find(|&position| {
                plans[start..position]
                    .iter()
                    .any(|earlier| variants.follows(plans[position].index, earlier.index))
            })
            .unwrap_or(full);
        let chunk = &plans[start..end];
        outcome
            .written_paths
            .extend(chunk.iter().map(|plan| plan.written_path.clone()));
        let context = glib::MainContext::default();
        let tasks = chunk
            .iter()
            .cloned()
            .map(|plan| {
                let (cancellable, copying, earlier) = (
                    cancellable.clone(),
                    copying.clone(),
                    variants.earlier(plan.index),
                );
                context.spawn_local(async move {
                    let target = gio::File::for_path(&plan.written_path);
                    let taken = if case_variants::taken_by(&target, earlier).await {
                        Taken::Number
                    } else {
                        Taken::Fail
                    };
                    transfer_planned(&plan, taken, moving, &cancellable, &copying)
                        .await
                        .map(|transferred| (transferred, taken))
                })
            })
            .collect::<Vec<_>>();
        let mut renamed = Vec::new();
        for (plan, task) in chunk.iter().zip(tasks) {
            match task.await {
                Ok(Ok((transferred, taken))) => {
                    variants.saved(plan.index, &transferred.target).await;
                    let aliased = taken == Taken::Number;
                    let cause = plan.renamed.or(aliased.then_some(RenameCause::Alias));
                    if let (Some(cause), Some(name)) = (cause, plan.source_path.file_name()) {
                        let requested = gio::File::for_path(plan.written_path.with_file_name(name));
                        let saved_as = if aliased {
                            transferred.target.clone()
                        } else {
                            gio::File::for_path(&plan.written_path)
                        };
                        renamed.extend(renamed_item(&requested, &saved_as, cause));
                    }
                    if let Some(progress) = &copying.progress {
                        progress.finish_item(
                            transferred.reported,
                            item_sizes[plan.index],
                            item_files[plan.index],
                            (!moving)
                                .then(|| location_for_file(&transferred.target))
                                .flatten(),
                        );
                    }
                    let done = Done {
                        index: plan.index,
                        moved_to: location_for_file(&transferred.target),
                    };
                    let copied = transferred.reported.bytes;
                    let is_last = Some(plan.index) == last;
                    settling.owe(done, transferred.owed, copied, is_last).await;
                }
                Ok(Err(error)) => outcome.failed.push((plan.index, error)),
                Err(_) => outcome
                    .failed
                    .push((plan.index, io_error("Copy worker failed"))),
            }
        }
        if let Some(progress) = &copying.progress {
            progress.report(TransferReport {
                renamed,
                ..TransferReport::default()
            });
        }
        outcome.next = end;
        if !outcome.failed.is_empty() || settling.failure().is_some() {
            break;
        }
    }
    outcome
}

async fn transfer_planned(
    plan: &ParallelCopyPlan,
    taken: Taken,
    moving: bool,
    cancellable: &gio::Cancellable,
    copying: &Copying,
) -> Result<Transferred, glib::Error> {
    if moving {
        let source = gio::File::for_path(&plan.source_path);
        let target = gio::File::for_path(&plan.written_path);
        return move_owing(source, target, taken, cancellable.clone(), copying.clone()).await;
    }
    let options = copy_engine::Options {
        names: copying.names.clone(),
        ..copy_engine::Options::default()
    };
    let copied = copy_engine::copy_local(
        plan.source_path.clone(),
        copy_engine::Destination::New(plan.written_path.clone(), taken),
        options,
        copying.progress.clone(),
        cancellable,
    )
    .await?;
    Ok(Transferred {
        target: gio::File::for_path(plan.written_path.with_file_name(copied.name)),
        reported: copied.reported,
        owed: Owed::default(),
    })
}

async fn first_free_target<T, F: Future<Output = Result<T, glib::Error>>>(
    target: &gio::File,
    taken: Taken,
    is_directory: bool,
    cancellable: &gio::Cancellable,
    mut attempt: impl FnMut(gio::File) -> F,
) -> Result<(gio::File, T), glib::Error> {
    if taken == Taken::Fail {
        return attempt(target.clone())
            .await
            .map(|value| (target.clone(), value));
    }
    if let (Some(parent), Some(name)) = (target.parent(), target.basename()) {
        let limit = mount_profile::folder_name_limit(&parent).await;
        for name in numbered_names(name.as_os_str(), is_directory, limit) {
            cancellable.set_error_if_cancelled()?;
            let candidate = parent.child(&name);
            if query_exists(&candidate, cancellable).await {
                continue;
            }
            match attempt(candidate.clone()).await {
                Err(error) if error.matches(gio::IOErrorEnum::Exists) => {}
                result => return result.map(|value| (candidate, value)),
            }
        }
    }
    cancellable.set_error_if_cancelled()?;
    Err(translated_io_error(
        "Could not find an unused duplicate name",
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopyInto {
    New,
    Existing,
}

impl CopyInto {
    fn file_flags(self) -> gio::FileCopyFlags {
        let overwrite = if self == Self::Existing {
            gio::FileCopyFlags::OVERWRITE
        } else {
            gio::FileCopyFlags::NONE
        };
        gio::FileCopyFlags::ALL_METADATA | overwrite
    }

    async fn existing_folder(
        self,
        target: &gio::File,
        cancellable: &gio::Cancellable,
    ) -> Option<ExistingFolder> {
        if self == Self::New {
            return None;
        }
        let info = await_cancellable(target, cancellable, |target, cancellable, result| {
            target.query_info_async(
                "standard::type,unix::device",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await
        .ok()?;
        Some(ExistingFolder {
            device: device(target, &info),
        })
    }
}

#[derive(Clone, Copy)]
struct ExistingFolder {
    device: Option<u64>,
}

fn device(file: &gio::File, info: &gio::FileInfo) -> Option<u64> {
    let device = (file.is_native() && info.has_attribute(gio::FILE_ATTRIBUTE_UNIX_DEVICE))
        .then(|| u64::from(info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_DEVICE)));
    #[cfg(test)]
    let device = tests::fs_quirks::gio_device(file, device);
    device
}

#[derive(Clone, Copy)]
struct TargetFolder {
    names: FolderNames,
    devices: (Option<u64>, Option<u64>),
}

#[derive(Clone, Default)]
struct GioNames {
    policy: NamePolicy,
    parent: Option<TargetFolder>,
    left: LeftOut,
}

impl GioNames {
    fn new(policy: NamePolicy, left: LeftOut) -> Self {
        Self {
            policy,
            parent: None,
            left,
        }
    }

    async fn of(
        &self,
        (source, source_device): (&gio::File, Option<u64>),
        target: &gio::File,
        existing: Option<ExistingFolder>,
        cancellable: &gio::Cancellable,
    ) -> Result<TargetFolder, glib::Error> {
        let devices = (source_device, existing.and_then(|existing| existing.device));
        match (self.parent, existing) {
            (Some(parent), None) => Ok(parent),
            (Some(parent), Some(_)) => {
                let folders = (source, target);
                existing_folder_names(&self.policy, parent, folders, devices, cancellable).await
            }
            (None, _) => Ok(TargetFolder {
                names: self.policy.folder(source, target, cancellable).await?,
                devices,
            }),
        }
    }

    fn within(&self, folder: TargetFolder) -> Self {
        Self {
            policy: self.policy.clone(),
            parent: Some(folder),
            left: self.left.clone(),
        }
    }
}

#[derive(Clone, Default)]
struct LeftOut {
    refused: Rc<Cell<bool>>,
    links: Rc<RefCell<Vec<(gio::File, gio::File)>>>,
}

impl LeftOut {
    fn add(&self, source: gio::File, target: gio::File) {
        self.links.borrow_mut().push((source, target));
    }

    fn sources(&self) -> Vec<gio::File> {
        self.links
            .borrow()
            .iter()
            .map(|(source, _)| source.clone())
            .collect()
    }

    fn report(&self, progress: Option<&TransferProgressTracker>) {
        let Some(progress) = progress else {
            return;
        };
        let skipped = self
            .links
            .borrow()
            .iter()
            .filter_map(|(_, target)| {
                Some(SkippedItem {
                    location: location_for_file(target)?,
                    kind: SpecialKind::Symlink,
                })
            })
            .collect::<Vec<_>>();
        if !skipped.is_empty() {
            progress.report(TransferReport {
                skipped,
                ..TransferReport::default()
            });
        }
    }

    fn keep_in(&self, sources: &copy_engine::Manifest, source: &Path) {
        let Some(parent) = source.parent() else {
            return;
        };
        for (link, _) in self.links.borrow().iter() {
            if let Some(path) = link.path()
                && let Ok(path) = path.strip_prefix(parent)
            {
                sources.keep(path, KeptCause::Skipped(SpecialKind::Symlink));
            }
        }
    }
}

/// Looked up again only where another filesystem is mounted at either, as a bind mount of the
/// parent shares its rules.
async fn existing_folder_names(
    policy: &NamePolicy,
    parent: TargetFolder,
    (source, target): (&gio::File, &gio::File),
    devices: (Option<u64>, Option<u64>),
    cancellable: &gio::Cancellable,
) -> Result<TargetFolder, glib::Error> {
    if target.is_native() && (devices.1.is_none() || devices != parent.devices) {
        Ok(TargetFolder {
            names: policy.folder(source, target, cancellable).await?,
            devices,
        })
    } else {
        Ok(parent)
    }
}

async fn query_exists(file: &gio::File, cancellable: &gio::Cancellable) -> bool {
    await_cancellable(file, cancellable, |file, cancellable, result| {
        file.query_info_async(
            gio::FILE_ATTRIBUTE_STANDARD_TYPE,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
    .is_ok()
}

async fn next_children(
    enumerator: &gio::FileEnumerator,
    cancellable: &gio::Cancellable,
) -> Result<Vec<gio::FileInfo>, glib::Error> {
    await_cancellable(
        enumerator,
        cancellable,
        |enumerator, cancellable, result| {
            enumerator.next_files_async(
                64,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        },
    )
    .await
}

fn renamed_item(
    requested: &gio::File,
    saved_as: &gio::File,
    cause: RenameCause,
) -> Option<RenamedItem> {
    Some(RenamedItem {
        requested: location_for_file(requested)?,
        saved_as: location_for_file(saved_as)?,
        cause,
    })
}

/// Children whose names are fixed come once the folder is listed, numbered past every sibling.
/// Where the mount takes two names for one, the whole folder is listed first.
struct NamedChildren<'a> {
    children: Children<'a>,
    names: FolderNames,
    kept: Vec<OsString>,
    held: Vec<(OsString, gio::FileType)>,
    listed: bool,
}

enum Children<'a> {
    Enumerated(&'a gio::FileEnumerator),
    Listed(Vec<(OsString, gio::FileType)>),
}

impl<'a> NamedChildren<'a> {
    fn new(enumerator: &'a gio::FileEnumerator, names: FolderNames) -> Self {
        Self::of(Children::Enumerated(enumerator), names)
    }

    fn listed(children: Vec<(OsString, gio::FileType)>, names: FolderNames) -> Self {
        Self::of(Children::Listed(children), names)
    }

    fn of(children: Children<'a>, names: FolderNames) -> Self {
        Self {
            children,
            names,
            kept: Vec::new(),
            held: Vec::new(),
            listed: false,
        }
    }

    async fn batch(
        &mut self,
        cancellable: &gio::Cancellable,
    ) -> Result<Vec<(OsString, gio::FileType)>, glib::Error> {
        match &mut self.children {
            Children::Enumerated(enumerator) => Ok(next_children(enumerator, cancellable)
                .await?
                .iter()
                .map(|child| (child.name().into_os_string(), child.file_type()))
                .collect()),
            Children::Listed(children) => Ok(std::mem::take(children)),
        }
    }

    async fn next(
        &mut self,
        cancellable: &gio::Cancellable,
    ) -> Result<Vec<(OsString, gio::FileType, Planned)>, glib::Error> {
        let kept = |name: OsString| Planned {
            target: name,
            cause: None,
        };
        while !self.listed {
            let batch = self.batch(cancellable).await?;
            self.listed = batch.is_empty();
            let children = batch.into_iter();
            if !self.names.fixing {
                return Ok(children
                    .map(|(name, kind)| (name.clone(), kind, kept(name)))
                    .collect());
            }
            if self.names.rules.folds {
                self.held.extend(children);
                continue;
            }
            let mut ready = Vec::new();
            for (name, kind) in children {
                let is_directory = kind == gio::FileType::Directory;
                if fix_name(&name, &self.names.rules, is_directory)
                    .cause
                    .is_some()
                {
                    self.held.push((name, kind));
                } else {
                    self.kept.push(name.clone());
                    ready.push((name.clone(), kind, kept(name)));
                }
            }
            if !ready.is_empty() {
                return Ok(ready);
            }
        }
        let held = std::mem::take(&mut self.held);
        if held.is_empty() {
            return Ok(Vec::new());
        }
        let entries = self
            .kept
            .iter()
            .map(|name| PlanEntry {
                name,
                is_directory: false,
                fixing: false,
                repeatable: false,
            })
            .chain(held.iter().map(|(name, kind)| PlanEntry {
                name,
                is_directory: *kind == gio::FileType::Directory,
                fixing: true,
                repeatable: false,
            }))
            .collect::<Vec<_>>();
        let planned = plan_names(&entries, &self.names.rules, Key::Folded)
            .map_err(placement_error)?
            .into_iter()
            .skip(self.kept.len());
        Ok(held
            .into_iter()
            .zip(planned)
            .map(|((name, kind), planned)| (name, kind, planned))
            .collect())
    }
}

fn was_cancelled(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled)
}

/// A cancelled copy between local folders removes what it wrote, or fails; GIO's copies to or
/// from other locations are not checked.
fn undone_when_cancelled(source: &gio::File, target: &gio::File) -> bool {
    source.is_native() && target.is_native()
}

/// GLib checks the cancel only before it moves a local item to Trash, while a permanent delete
/// can remove a file, or part of a folder, before it sees the cancel.
fn deletion_undone_when_cancelled(location: &Location, permanent: bool) -> bool {
    location.native_path().is_some() && !permanent
}

fn is_trash_unsupported_failure(permanent: bool, error: &TrashError) -> bool {
    !permanent && error.is_unsupported()
}

/// GIO deletes a directory only when it is empty.
async fn remove_empty_directory(
    file: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    await_cancellable(file, cancellable, |file, cancellable, result| {
        file.delete_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
            result.resolve(output)
        });
    })
    .await
}

fn cancelled_local_operation() -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Cancelled, "Operation cancelled")
}

async fn run_local_fs_step<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, glib::Error> {
    run_blocking(move || work().map_err(io_error)).await
}

struct CreatedCopyRoot {
    was_created: Cell<bool>,
    identity: Cell<Option<Recorded>>,
}

impl CreatedCopyRoot {
    fn new() -> Self {
        Self {
            was_created: Cell::new(false),
            identity: Cell::new(None),
        }
    }
}

/// The folder is made without the copy's cancellable: GIO and GVfs can report
/// a cancel after making it, which would leave it behind.
async fn make_copy_folder(
    target: &gio::File,
    cancellable: &gio::Cancellable,
    created_root: &Option<Rc<CreatedCopyRoot>>,
) -> Result<(), glib::Error> {
    cancellable.set_error_if_cancelled()?;
    let folder = target.clone();
    run_blocking(move || folder.make_directory(gio::Cancellable::NONE))
        .await
        .map_err(|error| {
            if cancellable.is_cancelled() {
                cancelled_local_operation()
            } else {
                error
            }
        })?;
    if let Some(created_root) = created_root {
        created_root.was_created.set(true);
        created_root
            .identity
            .set(local_file_identity(target).await?);
    }
    Ok(())
}

type RemoteFileStageCopy = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

type RemoteFileStageCommit = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

async fn discard_incomplete_copy(stage: gio::File) -> Result<(), glib::Error> {
    match permanently_delete(stage, false, gio::Cancellable::new()).await {
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(()),
        result => result,
    }
}

/// Reserves an unpredictable sibling so a partial remote file is never exposed at the final path.
async fn create_remote_file_stage(
    target: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<gio::File, glib::Error> {
    let parent = target
        .parent()
        .ok_or_else(|| translated_io_error("The destination has no parent directory"))?;
    let stage = parent.child(HiddenName::Copy.fresh());
    let stream = match await_cancellable(&stage, cancellable, |stage, cancellable, result| {
        stage.create_async(
            gio::FileCreateFlags::PRIVATE,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
    {
        Ok(stream) => stream,
        Err(error) if was_cancelled(&error) => {
            let cleanup = discard_incomplete_copy(stage).await;
            return Err(copy_failure_after_cleanup(error, cleanup));
        }
        Err(error) => return Err(error),
    };
    if let Err(error) = await_cancellable(&stream, cancellable, |stream, cancellable, result| {
        stream.close_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
            result.resolve(output)
        });
    })
    .await
    {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    Ok(stage)
}

fn copy_failure_after_cleanup(
    copy_error: glib::Error,
    cleanup_result: Result<(), glib::Error>,
) -> glib::Error {
    match cleanup_result {
        Ok(()) => copy_error,
        Err(cleanup_error) => io_error(rust_i18n::t!(
            "%{error}; the incomplete copy could not be removed: %{cleanup_error}",
            error = crate::services::gio_error_message(&copy_error),
            cleanup_error = crate::services::gio_error_detail(&cleanup_error)
        )),
    }
}

async fn copy_new_remote_file_with(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    copy_to_stage: RemoteFileStageCopy,
    commit_stage: RemoteFileStageCommit,
) -> Result<gio::File, glib::Error> {
    let stage = create_remote_file_stage(&target, &cancellable).await?;
    if let Err(error) = copy_to_stage(source, stage.clone(), cancellable.clone()).await {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    if let Err(error) = cancellable.set_error_if_cancelled() {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    let committed = first_free_target(&target, taken, false, &cancellable, |target| {
        commit_stage(stage.clone(), target, cancellable.clone())
    })
    .await;
    match committed {
        Ok((target, ())) => Ok(target),
        Err(error) => {
            let cleanup = discard_incomplete_copy(stage).await;
            Err(copy_failure_after_cleanup(error, cleanup))
        }
    }
}

#[derive(Clone)]
enum CopyRecord {
    Nothing,
    Root(Rc<CreatedCopyRoot>),
}

fn copy_recursively_with_progress(
    source: gio::File,
    target: gio::File,
    into: CopyInto,
    cancellable: gio::Cancellable,
    record: CopyRecord,
    progress: Option<Rc<TransferProgressTracker>>,
    names: GioNames,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let info = await_cancellable(&source, &cancellable, |source, cancellable, result| {
            source.query_info_async(
                "standard::type,standard::size,unix::device",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        if info.file_type() == gio::FileType::Directory {
            let existing = into.existing_folder(&target, &cancellable).await;
            if existing.is_none() {
                let created_root = match &record {
                    CopyRecord::Root(root) => Some(root.clone()),
                    _ => None,
                };
                make_copy_folder(&target, &cancellable, &created_root).await?;
            }
            let source_folder = (&source, device(&source, &info));
            let folder = names
                .of(source_folder, &target, existing, &cancellable)
                .await?;
            let record = match record {
                CopyRecord::Root(_) => CopyRecord::Nothing,
                record => record,
            };
            let enumerator = gio_close::enumerate_children(
                &source,
                "standard::name,standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                Some(&cancellable),
            )
            .await?;
            let names = names.within(folder);
            let mut renamed = Vec::new();
            let mut listed = NamedChildren::new(&enumerator, folder.names);
            let copied = async {
                loop {
                    let children = listed.next(&cancellable).await?;
                    if children.is_empty() {
                        return Ok::<_, glib::Error>(());
                    }
                    for (name, kind, planned) in children {
                        let link = kind == gio::FileType::SymbolicLink;
                        if link && (!folder.names.links || names.left.refused.get()) {
                            names.left.add(source.child(&name), target.child(&name));
                            continue;
                        }
                        let copied = copy_recursively_with_progress(
                            source.child(&name),
                            target.child(&planned.target),
                            into,
                            cancellable.clone(),
                            record.clone(),
                            progress.clone(),
                            names.clone(),
                        )
                        .await;
                        if let Err(error) = &copied
                            && link
                            && unheld(error).is_some()
                        {
                            names.left.refused.set(true);
                            names.left.add(source.child(&name), target.child(&name));
                            continue;
                        }
                        let saved_as = target.child(&planned.target);
                        if let Some(cause) = planned.cause
                            && (copied.is_ok() || query_exists(&saved_as, &cancellable).await)
                        {
                            renamed.extend(renamed_item(&target.child(&name), &saved_as, cause));
                        }
                        copied?;
                    }
                }
            }
            .await;
            if let Some(progress) = &progress {
                progress.report(TransferReport {
                    renamed,
                    ..TransferReport::default()
                });
            }
            copied
        } else {
            let source_size = info
                .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_SIZE)
                .then(|| info.size().max(0) as u64);
            let flags = into.file_flags() | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
            let track_bytes = source_size.is_none_or(|size| size >= BYTE_PROGRESS_MIN_FILE_SIZE);
            let file_progress = progress.as_ref().map(|progress| {
                progress.begin_file(
                    transfer_source_name(&source)
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    track_bytes,
                )
            });
            let reporter = file_progress
                .as_ref()
                .and_then(FileTransferProgress::reporter);
            let link = (info.file_type() == gio::FileType::SymbolicLink).then(|| source.clone());
            let result = copy_file_with_progress(
                source,
                target.clone(),
                flags,
                cancellable.clone(),
                reporter,
            )
            .await
            .map_err(|error| match link {
                // SMB and MTP backends refuse links.
                Some(link) if error.matches(gio::IOErrorEnum::NotSupported) => {
                    let message = rust_i18n::t!(
                        "Could not copy %{name}: %{error}",
                        name = transfer_source_name(&link)
                            .unwrap_or_default()
                            .to_string_lossy(),
                        error = crate::services::gio_error_detail(&error)
                    );
                    glib::Error::new(Unheld::Symlink, &message)
                }
                _ => error,
            });
            if result.is_ok()
                && let Some(file_progress) = file_progress
            {
                file_progress.finish(source_size);
            }
            result
        }
    })
}

#[cfg(test)]
fn copy_recursively(
    source: gio::File,
    target: gio::File,
    merge: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if merge {
            return merge_copy(None, NamePolicy::default())(
                source,
                target,
                MergeCopy::default(),
                cancellable,
            )
            .await
            .map(drop);
        }
        copy_new_recursively_on_filesystem(
            source,
            target,
            Taken::Fail,
            cancellable,
            None,
            NamePolicy::default(),
            None,
        )
        .await
        .map(drop)
    })
}

async fn copy_new_recursively_on_filesystem(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
    left: Option<LeftOut>,
) -> Result<gio::File, glib::Error> {
    copy_new_with_ownership(source, target, taken, cancellable, progress, names, left)
        .await
        .map(|(target, _)| target)
}

async fn copy_new_with_ownership(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
    left: Option<LeftOut>,
) -> Result<(gio::File, Option<copy_engine::Ownership>), glib::Error> {
    if source.is_native()
        && target.is_native()
        && let (Some(source_path), Some(target_path)) = (source.path(), target.path())
    {
        let options = copy_engine::Options {
            names,
            ..copy_engine::Options::default()
        };
        let copied = copy_engine::copy_local(
            source_path,
            copy_engine::Destination::New(target_path.clone(), taken),
            options,
            progress,
            &cancellable,
        )
        .await?;
        return Ok((
            gio::File::for_path(target_path.with_file_name(copied.name)),
            copied.ownership,
        ));
    }

    let source_type = if !target.is_native() || taken == Taken::Number {
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
        Some(info.file_type())
    } else {
        None
    };
    let is_directory = source_type == Some(gio::FileType::Directory);
    if !target.is_native() && !is_directory {
        let copy_progress = progress.clone();
        let copy_names = names.clone();
        return copy_new_remote_file_with(
            source,
            target,
            taken,
            cancellable,
            Rc::new(move |source, stage, cancellable| {
                let progress = copy_progress.clone();
                let names = GioNames::new(copy_names.clone(), LeftOut::default());
                Box::pin(async move {
                    copy_recursively_with_progress(
                        source,
                        stage,
                        CopyInto::Existing,
                        cancellable,
                        CopyRecord::Nothing,
                        progress,
                        names,
                    )
                    .await
                })
            }),
            Rc::new(|stage, target, cancellable| {
                Box::pin(async move {
                    await_cancellable(&stage, &cancellable, move |stage, cancellable, result| {
                        stage.move_async(
                            &target,
                            gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE,
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
        .map(|target| (target, None));
    }

    let left_out = left.clone().unwrap_or_default();
    let copied = first_free_target(&target, taken, is_directory, &cancellable, |target| {
        let (source, cancellable, progress) =
            (source.clone(), cancellable.clone(), progress.clone());
        let names = GioNames::new(names.clone(), left_out.clone());
        async move {
            let created_root = Rc::new(CreatedCopyRoot::new());
            let result = copy_recursively_with_progress(
                source,
                target.clone(),
                CopyInto::New,
                cancellable.clone(),
                CopyRecord::Root(created_root.clone()),
                progress,
                names,
            )
            .await;
            match result {
                Err(error) if was_cancelled(&error) && created_root.was_created.get() => {
                    let cleanup_result = permanently_delete_maybe_local_if_unchanged(
                        target,
                        true,
                        created_root.identity.get(),
                        gio::Cancellable::new(),
                    )
                    .await;
                    Err(copy_failure_after_cleanup(error, cleanup_result))
                }
                // Taken inside a folder this copy made, where no other
                // name for the folder helps.
                Err(error)
                    if taken == Taken::Number
                        && error.matches(gio::IOErrorEnum::Exists)
                        && created_root.was_created.get() =>
                {
                    Err(io_error(crate::services::gio_error_message(&error)))
                }
                result => result,
            }
        }
    })
    .await?;
    if left.is_none() {
        left_out.report(progress.as_deref());
    }
    Ok((copied.0, None))
}

enum Moved {
    Done(gio::File),
    Copy,
}

type MoveAttempt = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        Taken,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<Moved, glib::Error>>>>,
>;

type RemoteCopy = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        Taken,
        gio::Cancellable,
        LeftOut,
    ) -> Pin<Box<dyn Future<Output = Result<gio::File, glib::Error>>>>,
>;

fn native_path(file: &gio::File) -> Option<PathBuf> {
    file.is_native().then(|| file.path()).flatten()
}

struct Transferred {
    target: gio::File,
    reported: Reported,
    owed: Owed,
}

async fn move_local_owing(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    copying: Copying,
    attempt_move: MoveAttempt,
    remote_copy: RemoteCopy,
) -> Result<Transferred, glib::Error> {
    if let Moved::Done(target) =
        attempt_move(source.clone(), target.clone(), taken, cancellable.clone()).await?
    {
        return Ok(Transferred {
            target,
            reported: Reported::default(),
            owed: Owed::default(),
        });
    }
    let Copying {
        progress, names, ..
    } = copying;
    let mut owed = Owed::default();
    if let Some(target_path) = native_path(&target) {
        owed.barrier = Some(copy_engine::barrier(split_local_path(&target_path)?.0).await?);
    }
    let (target, reported, deletion) = match (native_path(&source), native_path(&target)) {
        (Some(source_path), Some(target_path)) => {
            let options = copy_engine::Options {
                names,
                manifest: true,
                durable: true,
                ..copy_engine::Options::default()
            };
            let copied = copy_engine::copy_local(
                source_path,
                copy_engine::Destination::New(target_path.clone(), taken),
                options,
                progress.clone(),
                &cancellable,
            )
            .await?;
            let target = gio::File::for_path(target_path.with_file_name(copied.name));
            let deletion = match copied.manifest {
                Some(sources) => Deletion::Sources(source, sources),
                None => Deletion::Source(source, Vec::new()),
            };
            (target, copied.reported, deletion)
        }
        // Recorded before GIO copies it, so a change during the copy shows.
        (Some(source_path), None) => {
            let sources = copy_engine::snapshot(source_path.clone(), &cancellable).await?;
            let left = LeftOut::default();
            let target = remote_copy(
                source.clone(),
                target,
                taken,
                cancellable.clone(),
                left.clone(),
            )
            .await?;
            left.keep_in(&sources, &source_path);
            (
                target,
                Reported::default(),
                Deletion::Sources(source, sources),
            )
        }
        _ => {
            let left = LeftOut::default();
            let target = copy_new_recursively_on_filesystem(
                source.clone(),
                target,
                taken,
                cancellable.clone(),
                progress.clone(),
                names,
                Some(left.clone()),
            )
            .await?;
            if let Some(path) = native_path(&target)
                && owed
                    .barrier
                    .as_ref()
                    .is_some_and(copy_engine::Barrier::syncs_each_file)
            {
                copy_engine::sync_files(vec![path], &cancellable).await?;
            }
            let kept = left.sources();
            (target, Reported::default(), Deletion::Source(source, kept))
        }
    };
    owed.owe(deletion);
    Ok(Transferred {
        target,
        reported,
        owed,
    })
}

#[cfg(test)]
async fn move_local_with_progress(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    copying: Copying,
    attempt_move: MoveAttempt,
    remote_copy: RemoteCopy,
) -> Result<gio::File, glib::Error> {
    let progress = copying.progress.clone();
    let moved = move_local_owing(
        source,
        target,
        taken,
        cancellable.clone(),
        copying,
        attempt_move,
        remote_copy,
    )
    .await?;
    moved.owed.settle(progress.as_deref(), &cancellable).await?;
    Ok(moved.target)
}

async fn delete_moved_sources(
    source: &gio::File,
    sources: copy_engine::Manifest,
    unlinks: Arc<copy_engine::Unlinks>,
    progress: Option<&TransferProgressTracker>,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let (kept, deleted) =
        copy_engine::delete_unchanged_sources(sources, unlinks, cancellable).await;
    if let (Some(progress), Some(parent)) =
        (progress, source.path().as_deref().and_then(Path::parent))
    {
        progress.report(TransferReport {
            kept: kept_items(parent, kept),
            ..TransferReport::default()
        });
    }
    deleted
}

fn kept_items(parent: &Path, kept: Vec<(PathBuf, KeptCause)>) -> Vec<KeptItem> {
    kept.into_iter()
        .map(|(path, cause)| KeptItem {
            location: Location::local(parent.join(path)),
            cause,
        })
        .collect()
}

#[cfg(test)]
async fn move_local_with(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    attempt_move: MoveAttempt,
) -> Result<gio::File, glib::Error> {
    move_local_with_progress(
        source,
        target,
        Taken::Fail,
        cancellable,
        Copying::default(),
        attempt_move,
        remote_copy(Copying::default()),
    )
    .await
}

/// Standalone reason for a placement failure, for a `…: %{error}` slot.
fn placement_reason(error: PlaceError) -> String {
    match error {
        PlaceError::Exists => crate::i18n::tr("An item with that name already exists"),
        PlaceError::InvalidName => crate::i18n::tr(INVALID_NAME_REASON),
        PlaceError::IntoItself => crate::i18n::tr("Can\u{2019}t put a folder inside itself"),
        PlaceError::Changed => {
            crate::i18n::tr("The target changed while the operation was in progress")
        }
        PlaceError::Unconfirmed => crate::i18n::tr(UNCONFIRMED_REASON),
        PlaceError::CrossDevice => os_error_text(rustix::io::Errno::XDEV),
        PlaceError::Unsupported => crate::i18n::tr(UNSUPPORTED_REASON),
        PlaceError::Io(error) => os_error_text(error),
    }
}

fn placement_error_kind(error: PlaceError) -> gio::IOErrorEnum {
    match error {
        PlaceError::Exists => gio::IOErrorEnum::Exists,
        PlaceError::InvalidName => gio::IOErrorEnum::InvalidFilename,
        _ => gio::IOErrorEnum::Failed,
    }
}

/// A placement failure as a complete message about `name`, using `template`
/// (with `%{name}` and `%{error}`) unless the failure has its own sentence.
fn placement_failure(
    error: PlaceError,
    name: &str,
    template: fn(&str, &str) -> String,
) -> glib::Error {
    let message = match error {
        PlaceError::IntoItself => placement_reason(error),
        PlaceError::Changed => rust_i18n::t!(
            "%{name} changed while the operation was in progress",
            name = name
        )
        .into_owned(),
        error => template(
            name,
            &crate::services::error_detail(placement_reason(error)),
        ),
    };
    io_error(message)
}

fn could_not_move(name: &str, error: &str) -> String {
    rust_i18n::t!(
        "Could not move %{name}: %{error}",
        name = name,
        error = error
    )
    .into_owned()
}

async fn rename_native(
    source: PathBuf,
    target: PathBuf,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let (Some(source_parent), Some(source_name), Some(target_parent), Some(target_name)) = (
        source.parent().map(Path::to_path_buf),
        source.file_name().map(OsStr::to_os_string),
        target.parent().map(Path::to_path_buf),
        target.file_name().map(OsStr::to_os_string),
    ) else {
        return Err(translated_io_error("Invalid move destination"));
    };
    let stopped = cancellable.clone();
    run_local_fs_step(move || {
        let source_dir = open_local_parent_directory(&source_parent)?;
        let target_dir = if target_parent == source_parent {
            None
        } else {
            Some(open_local_parent_directory(&target_parent)?)
        };
        let from = Entry::new(&source_dir, &source_name);
        let to = Entry::new(target_dir.as_ref().unwrap_or(&source_dir), &target_name);
        let Ok(renamed) = deferred_close::retry_folder_rename(
            fs(),
            from,
            &Deadline::default(),
            &|| stopped.is_cancelled(),
            |failure| {
                matches!(
                    failure,
                    RenameFailure::Place(PlaceError::Io(rustix::io::Errno::ACCESS))
                )
            },
            || placement::rename_in_place(fs(), from, to),
        ) else {
            return Ok(Err(cancelled_local_operation()));
        };
        Ok(renamed.map(drop).map_err(|failure| match failure {
            RenameFailure::Place(error) => still_open_rename(from, error)
                .map_or_else(|| rename_failure_error(failure, &source, &target), io_error),
            failure => rename_failure_error(failure, &source, &target),
        }))
    })
    .await?
}

fn rename_failure_error(failure: RenameFailure, source: &Path, target: &Path) -> glib::Error {
    let name = |path: &Path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    match failure {
        RenameFailure::Place(PlaceError::Exists) => glib::Error::new(
            gio::IOErrorEnum::Exists,
            &rust_i18n::t!("“%{name}” already exists", name = name(target)),
        ),
        RenameFailure::Place(error) => placement_error(error),
        RenameFailure::Stranded { temporary, error } => io_error(rust_i18n::t!(
            "Could not rename “%{name}”; it was left as “%{temporary}”: %{error}",
            name = name(source),
            temporary = temporary.to_string_lossy(),
            error = crate::services::error_detail(placement_reason(error))
        )),
    }
}

fn move_local_path(
    source_path: PathBuf,
    target_path: PathBuf,
    taken: Taken,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<Moved, glib::Error>>>> {
    Box::pin(async move {
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        let Some(source_parent_path) = source_path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error("Cannot move the filesystem root"));
        };
        let Some(source_name) = source_path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid move source"));
        };
        let Some(target_parent_path) = target_path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error(
                "The move destination has no parent directory",
            ));
        };
        let Some(target_name) = target_path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid move destination"));
        };

        let display_name = source_name.to_string_lossy().into_owned();
        let stopped = cancellable.clone();
        run_local_fs_step(move || {
            let source_parent = open_local_parent_directory(&source_parent_path)?;
            let target_parent = open_local_parent_directory(&target_parent_path)?;
            let from = Entry::new(&source_parent, &source_name);
            let moved = || {
                let is_directory = match taken {
                    Taken::Fail => false,
                    Taken::Number => match fs().stat(from, false) {
                        Ok(stat) => stat.is_directory(),
                        Err(error) => return Ok(Err(PlaceError::Io(error))),
                    },
                };
                let limit = || mount_profile::name_limit(fs(), target_parent.as_fd());
                let deadline = Deadline::default();
                for name in taken.names(&target_name, is_directory, limit) {
                    let to = Entry::new(&target_parent, &name);
                    let placed = deferred_close::retry_folder_rename(
                        fs(),
                        from,
                        &deadline,
                        &|| stopped.is_cancelled(),
                        |error| matches!(error, PlaceError::Io(rustix::io::Errno::ACCESS)),
                        || placement::place(fs(), from, to, None),
                    )?;
                    if taken == Taken::Number && matches!(placed, Err(PlaceError::Exists)) {
                        continue;
                    }
                    return Ok(match placed {
                        Ok(_) => Ok(Moved::Done(gio::File::for_path(
                            target_parent_path.join(&name),
                        ))),
                        Err(PlaceError::CrossDevice | PlaceError::Unsupported) => {
                            match fs().stat(from, false) {
                                Ok(stat)
                                    if stat.is_directory()
                                        && placement::contains(fs(), stat.identity, to.dir)
                                            != Some(false) =>
                                {
                                    Err(PlaceError::IntoItself)
                                }
                                Ok(_) => Ok(Moved::Copy),
                                Err(error) => Err(PlaceError::Io(error)),
                            }
                        }
                        Err(error) => Err(error),
                    });
                }
                Ok(Err(PlaceError::Exists))
            };
            Ok(match moved() {
                Ok(moved) => moved.map_err(|error| match still_open_rename(from, error) {
                    Some(reason) => io_error(could_not_move(
                        &display_name,
                        &crate::services::error_detail(reason),
                    )),
                    None => placement_failure(error, &display_name, could_not_move),
                }),
                Err(WaitCancelled) => Err(cancelled_local_operation()),
            })
        })
        .await?
    })
}

fn still_open_rename(from: Entry<'_>, error: PlaceError) -> Option<String> {
    match error {
        PlaceError::Io(errno) => holders::refused_rename(fs(), from, errno),
        _ => None,
    }
}

#[cfg(test)]
async fn move_local(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
) -> Result<gio::File, glib::Error> {
    let copying = Copying {
        progress: progress.clone(),
        names,
        ..Copying::default()
    };
    move_local_with_progress(
        source,
        target,
        taken,
        cancellable,
        copying.clone(),
        attempt_move(progress),
        remote_copy(copying),
    )
    .await
}

async fn move_owing(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    copying: Copying,
) -> Result<Transferred, glib::Error> {
    let (attempt, remote) = (
        attempt_move(copying.progress.clone()),
        remote_copy(copying.clone()),
    );
    move_local_owing(source, target, taken, cancellable, copying, attempt, remote).await
}

fn attempt_move(progress: Option<Rc<TransferProgressTracker>>) -> MoveAttempt {
    Rc::new(move |source, target, taken, cancellable| {
        if let (Some(source_path), Some(target_path)) = (native_path(&source), native_path(&target))
        {
            return move_local_path(source_path, target_path, taken, cancellable);
        }
        // GIO would delete a native source itself, without checking what it copied.
        if source.is_native() {
            return Box::pin(async { Ok(Moved::Copy) });
        }
        let move_progress = progress.as_ref().map(|progress| {
            progress.begin_file(
                transfer_source_name(&source)
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                true,
            )
        });
        Box::pin(async move {
            let is_directory = taken == Taken::Number
                && await_cancellable(&source, &cancellable, |source, cancellable, result| {
                    source.query_info_async(
                        "standard::type",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                })
                .await?
                .file_type()
                    == gio::FileType::Directory;
            let flags = gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
            let moved = first_free_target(&target, taken, is_directory, &cancellable, |target| {
                let reporter = move_progress
                    .as_ref()
                    .and_then(FileTransferProgress::reporter);
                move_file_with_progress(
                    source.clone(),
                    target,
                    flags,
                    cancellable.clone(),
                    reporter,
                )
            })
            .await;
            match moved {
                Ok((target, ())) => {
                    if let Some(move_progress) = move_progress {
                        move_progress.finish(None);
                    }
                    Ok(Moved::Done(target))
                }
                Err(error) if error.matches(gio::IOErrorEnum::WouldRecurse) => {
                    let (guard_source, guard_target) = (source.clone(), target.clone());
                    let within = run_local_fs_step(move || {
                        Ok(target_is_within_source(&guard_source, &guard_target))
                    })
                    .await?;
                    if within != Some(false) {
                        return Err(placement_error(PlaceError::IntoItself));
                    }
                    Ok(Moved::Copy)
                }
                Err(error) => Err(error),
            }
        })
    })
}

fn remote_copy(copying: Copying) -> RemoteCopy {
    Rc::new(move |source, target, taken, cancellable, left| {
        let Copying {
            progress, names, ..
        } = copying.clone();
        Box::pin(copy_new_recursively_on_filesystem(
            source,
            target,
            taken,
            cancellable,
            progress,
            names,
            Some(left),
        ))
    })
}

fn io_error(error: impl std::fmt::Display) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string())
}

fn os_io_error(error: impl Into<io::Error>) -> glib::Error {
    io_error(os_error_text(error))
}

fn translated_io_error(message: &str) -> glib::Error {
    io_error(crate::i18n::tr(message))
}

async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, glib::Error> + Send + 'static,
) -> Result<T, glib::Error> {
    gio::spawn_blocking(work)
        .await
        .map_err(|_| io_error("Local filesystem task panicked"))?
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailedAction {
    Delete,
    Trash,
    Restore,
}

/// `OperationEvent` carries display text, so the summary is localized here.
fn operation_error_summary(errors: &[String], action: FailedAction) -> String {
    let items = crate::i18n::count("items", errors.len());
    let mut summary = match action {
        FailedAction::Delete => rust_i18n::t!(
            "%{items} could not be deleted. The remaining items were processed.",
            items = items
        ),
        FailedAction::Trash => rust_i18n::t!(
            "%{items} could not be moved to Trash. The remaining items were processed.",
            items = items
        ),
        FailedAction::Restore => rust_i18n::t!(
            "%{items} could not be restored. The remaining items were processed.",
            items = items
        ),
    }
    .into_owned();
    for error in errors.iter().take(8) {
        summary.push_str("\n\n• ");
        summary.push_str(error);
    }
    if errors.len() > 8 {
        summary.push_str("\n\n");
        summary.push_str(&crate::i18n::count("more_items", errors.len() - 8));
    }
    summary
}

fn deletion_error_summary(errors: &[String], permanent: bool) -> String {
    operation_error_summary(
        errors,
        if permanent {
            FailedAction::Delete
        } else {
            FailedAction::Trash
        },
    )
}

/// The retry dialog offers the permanent delete, so an unsupported Trash
/// needs no instruction here.
fn deletion_error_message(name: &str, error: &TrashError) -> String {
    match error.failure {
        TrashFailure::Unsupported(_) => {
            rust_i18n::t!("%{name}: This location doesn't support Trash.", name = name)
        }
        TrashFailure::ReadOnly => {
            rust_i18n::t!("%{name}: This location is read-only.", name = name)
        }
        TrashFailure::NoSpace => {
            rust_i18n::t!("%{name}: Not enough space to move to Trash.", name = name)
        }
        TrashFailure::Cancelled | TrashFailure::NotFound | TrashFailure::Other => {
            return item_error(name, crate::services::gio_error_detail(&error.error));
        }
    }
    .into_owned()
}

fn item_error(name: &str, error: impl std::fmt::Display) -> String {
    rust_i18n::t!("%{name}: %{error}", name = name, error = error).into_owned()
}

fn operation_handle(cancellable: gio::Cancellable) -> LoadHandle {
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
                    message: crate::i18n::tr(message),
                    password_failure: None,
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
            if let Some(folder) = file.parent()
                && let Some(message) =
                    typed_names::refused_in(&folder, &request.new_name, false).await
            {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message,
                    password_failure: None,
                });
                return;
            }
            let item = request.entry.location.clone();
            let new_name = request.new_name.clone();
            let affected_locations = item.parent().into_iter().collect();
            let roots = release::natives([&item]);
            let operation = ActiveOperation::begin(release::parents(&roots));
            operation.release(roots, &operation_cancellable).await;
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
            let result = match request.entry.location.native_path() {
                Some(path) => match glib::filename_from_utf8(&request.new_name) {
                    Ok((name, _)) => {
                        rename_native(
                            path.to_path_buf(),
                            path.with_file_name(name),
                            &operation_cancellable,
                        )
                        .await
                    }
                    Err(error) => Err(glib::Error::new(
                        gio::IOErrorEnum::InvalidFilename,
                        &error.to_string(),
                    )),
                },
                None => set_display_name(&file, &request.new_name, &operation_cancellable)
                    .await
                    .map(drop),
            };
            match result {
                Ok(()) => emit(OperationEvent::Renamed {
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
                    message: if error.matches(gio::IOErrorEnum::Exists) {
                        rust_i18n::t!("“%{name}” already exists", name = new_name).into_owned()
                    } else {
                        crate::services::gio_error_message(&error)
                    },
                    password_failure: None,
                }),
            }
        });
        operation_handle(cancellable)
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        create_entry::start(
            request.id,
            request.parent,
            request.name,
            request.unique_name,
            true,
            emit,
        )
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        create_entry::start(
            request.id,
            request.parent,
            request.name,
            request.unique_name,
            false,
            emit,
        )
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let destination = gio_file_for_location(&request.destination);
            let request = in_copy_order(request, &destination);
            let mut affected_locations = HashSet::from([request.destination.clone()]);
            for parent in request.items.iter().filter_map(|item| item.source.parent()) {
                affected_locations.insert(parent);
            }
            let moved = if request.move_sources {
                release::natives(request.items.iter().map(|item| &item.source))
            } else {
                Vec::new()
            };
            let operation = Rc::new(ActiveOperation::begin(release::parents(&moved)));
            operation.include_folder(&request.destination);
            let sources = request
                .items
                .iter()
                .map(|item| gio_file_for_location(&item.source))
                .collect::<Vec<_>>();
            let names = NamePolicy::new(NameMode::Fix);
            let pasted_into = names
                .destination(&destination, &sources, &operation_cancellable)
                .await;
            let max_file_size = pasted_into.profile.max_file_size;
            let TransferSizes {
                sizes: item_sizes,
                total: total_bytes,
                files: item_files,
                total_files,
                kinds,
                filesystems,
            } = match transfer_sizes(&sources, &operation_cancellable, max_file_size).await {
                Ok(sizes) => sizes,
                Err(error) if was_cancelled(&error) => {
                    emit(cancelled_event(
                        request.id,
                        Vec::new(),
                        Vec::new(),
                        request
                            .items
                            .iter()
                            .map(|item| item.source.clone())
                            .collect(),
                        affected_locations,
                    ));
                    return;
                }
                // Too large for FAT32, which the error says.
                Err(error) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: Vec::new(),
                        message: error.message().to_owned(),
                    });
                    return;
                }
            };
            let rules = names.applied(pasted_into.profile.names, pasted_into.profile.strictness);
            let crossing = pasted_into.fixing(&filesystems);
            let duplicate = duplicates(&sources, &destination, request.move_sources);
            let planned = match planned_paste(
                &request.items,
                &sources,
                &rules,
                (&crossing, &duplicate),
                &kinds,
            ) {
                Ok(planned) => planned,
                Err(error) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: Vec::new(),
                        message: placement_reason(error),
                    });
                    return;
                }
            };
            let targets = planned
                .iter()
                .map(|planned| {
                    planned
                        .as_ref()
                        .map(|planned| destination.child(&planned.target))
                })
                .collect::<Vec<_>>();
            let targets_of = |conflict: TransferConflict| {
                request
                    .items
                    .iter()
                    .zip(&targets)
                    .zip(&duplicate)
                    .filter(move |((item, _), duplicate)| item.conflict == conflict && !**duplicate)
                    .filter_map(|((_, target), _)| target.as_ref())
            };
            operation
                .include_trees(targets_of(TransferConflict::Merge).filter_map(location_for_file));
            let replaced =
                targets_of(TransferConflict::ReplaceExisting).filter_map(gio::File::path);
            operation
                .release(
                    moved.into_iter().chain(replaced).collect(),
                    &operation_cancellable,
                )
                .await;
            let mut variants =
                CaseVariants::new(planned.iter().enumerate().filter_map(|(index, planned)| {
                    Some((index, planned.as_ref()?.target.clone()))
                }));
            let progress =
                TransferProgressTracker::new(request.id, total_bytes, total_files, emit.clone());
            progress.emit();
            let mut settling = Settling::paste(&request, emit.clone(), progress.clone());
            let copying = Copying {
                progress: Some(progress.clone()),
                names: names.clone(),
                ..Copying::default()
            };
            if let Some((plans, worker_count)) = parallel_copy_plans(
                &request,
                &sources,
                &destination,
                &planned,
                &kinds,
                pasted_into.profile.windows_disk(),
            )
            .await
            {
                for plan in &plans {
                    affected_locations.insert(request.items[plan.index].source.clone());
                    affected_locations.insert(plan.target_location.clone());
                }
                let outcome = execute_parallel_copy_plans(
                    &plans,
                    (&item_sizes, &item_files),
                    (worker_count, request.move_sources),
                    &operation_cancellable,
                    &copying,
                    (&mut settling, &mut variants),
                )
                .await;
                settling.finish().await;
                if !outcome.failed.is_empty()
                    || outcome.next < plans.len()
                    || settling.failure().is_some()
                {
                    let (cut_off, failures): (Vec<_>, Vec<_>) = outcome
                        .failed
                        .iter()
                        .partition(|(_, error)| was_cancelled(error));
                    let mut failed = settling.failed();
                    failed.extend(
                        failures
                            .iter()
                            .map(|(index, _)| request.items[*index].source.clone()),
                    );
                    let failure_error = failures.first().map(|(_, error)| error);
                    let (failure, skip_flush) = failure_error.map_or((None, false), |error| {
                        copy_failure_on_fat32(error, max_file_size.is_some())
                    });
                    stop_transfer(
                        &outcome.written_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed: settling.completed(),
                            failed,
                            not_attempted: cut_off
                                .iter()
                                .map(|(index, _)| &request.items[*index])
                                .chain(&request.items[outcome.next..])
                                .map(|item| item.source.clone())
                                .collect(),
                            affected_locations,
                            failure: joined_failures(failure, settling.failure()),
                            skip_flush,
                        },
                    )
                    .await;
                    return;
                }
                if !flush_removable_writes(
                    outcome.written_paths,
                    &operation_cancellable,
                    &emit,
                    request.id,
                    &settling.completed(),
                    affected_locations,
                )
                .await
                {
                    return;
                }
                emit(OperationEvent::Pasted {
                    request_id: request.id,
                    locations: settling.completed(),
                });
                return;
            }
            // Undo deletes what a paste with a merge copied beside it where there is no Trash.
            let records_copies = !request.move_sources
                && request
                    .items
                    .iter()
                    .any(|item| item.conflict == TransferConflict::Merge)
                && match native_path(&destination) {
                    Some(path) => !can_trash(path).await,
                    None => false,
                };
            let mut written_paths = Vec::new();
            for (index, item) in request.items.iter().enumerate() {
                if operation_cancellable.is_cancelled() || settling.failure().is_some() {
                    settling.finish().await;
                    stop_transfer(
                        &written_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed: settling.completed(),
                            failed: settling.failed(),
                            not_attempted: request.items[index..]
                                .iter()
                                .map(|item| item.source.clone())
                                .collect(),
                            affected_locations,
                            failure: settling.failure(),
                            skip_flush: false,
                        },
                    )
                    .await;
                    return;
                }
                let source = sources[index].clone();
                let started = progress.reported();
                let Some(target) = targets[index].clone() else {
                    settling.finish().await;
                    let flush_error = flush_written_roots(&written_paths, &emit, request.id)
                        .await
                        .err();
                    let flush_error = joined_failures(settling.failure(), flush_error);
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: match flush_error {
                            Some(error) => rust_i18n::t!(
                                "A clipboard item has no file name. %{error}",
                                error = error
                            )
                            .into_owned(),
                            None => crate::i18n::tr("A clipboard item has no file name"),
                        },
                        password_failure: None,
                    });
                    return;
                };
                let is_duplicate = !request.move_sources && source.equal(&target);
                let needs_unique_target =
                    is_duplicate || item.conflict == TransferConflict::KeepBoth;
                let is_noop = if is_duplicate {
                    item.conflict == TransferConflict::ReplaceExisting
                } else {
                    let (source, destination, target) =
                        (source.clone(), destination.clone(), target.clone());
                    gio::spawn_blocking(move || transfer_is_noop(&source, &destination, &target))
                        .await
                        .unwrap_or(false)
                };
                let last = index + 1 == request.items.len();
                if is_noop {
                    let done = Done {
                        index,
                        moved_to: None,
                    };
                    settling.owe(done, Owed::default(), 0, last).await;
                    progress.finish_item(
                        progress.reported().since(started),
                        item_sizes[index],
                        item_files[index],
                        None,
                    );
                    continue;
                }
                let mut taken = if needs_unique_target {
                    Taken::Number
                } else {
                    Taken::Fail
                };
                let mut aliased = false;
                affected_locations.insert(item.source.clone());
                let default_location = location_for_file(&target);
                if let Some(target) = default_location.clone() {
                    affected_locations.insert(target);
                }
                if let Some(path) = target.path() {
                    written_paths.push(path);
                }
                let replaced_permanently = Rc::new(Cell::new(false));
                let undone = item.conflict != TransferConflict::Merge
                    && undone_when_cancelled(&source, &target);
                let mut copied_ownership = None;
                // A destination known to hold no links is not asked to make
                // one; a move within its filesystem makes none.
                let result = if kinds[index] == Some(gio::FileType::SymbolicLink)
                    && pasted_into.profile.symlinks == Some(false)
                    && (crossing[index] || !request.move_sources)
                {
                    Err(glib::Error::new(Unheld::Symlink, ""))
                } else if is_duplicate {
                    copy_new_with_ownership(
                        source,
                        target,
                        taken,
                        operation_cancellable.clone(),
                        Some(progress.clone()),
                        names.clone(),
                        None,
                    )
                    .await
                    .map(|(target, ownership)| {
                        copied_ownership = ownership;
                        (target, Owed::default())
                    })
                } else if item.conflict == TransferConflict::Merge {
                    merge_owing(
                        source,
                        target.clone(),
                        request.move_sources,
                        &operation_cancellable,
                        Some(&mut affected_locations),
                        &copying.placed_entries,
                        progress_merge_hooks(
                            Some(progress.clone()),
                            names.clone(),
                            Some(operation.clone()),
                            &|plan| {
                                for (location, identity) in plan.originals {
                                    emit(OperationEvent::ItemTrashed {
                                        request_id: request.id,
                                        location,
                                        identity,
                                    });
                                }
                                emit(OperationEvent::Merged {
                                    request_id: request.id,
                                    source: item.source.clone(),
                                    created: plan.created,
                                    overwritten: plan.overwritten,
                                    placed: plan.placed,
                                });
                            },
                        ),
                    )
                    .await
                    .map(|owed| (target, owed))
                } else if item.conflict == TransferConflict::ReplaceExisting {
                    let replaced_target = default_location;
                    let emit = emit.clone();
                    let replaced_permanently = replaced_permanently.clone();
                    replace_owing(
                        source,
                        target.clone(),
                        request.move_sources,
                        &operation_cancellable,
                        Some(&mut affected_locations),
                        copying.clone(),
                        &move |original, placed| {
                            let identity = match original {
                                ReplacedOriginal::Trashed(identity) => identity,
                                ReplacedOriginal::Deleted => {
                                    replaced_permanently.set(true);
                                    return;
                                }
                            };
                            if let (Some(target), Some(identity)) = (&replaced_target, identity) {
                                emit(OperationEvent::ItemTrashed {
                                    request_id: request.id,
                                    location: target.clone(),
                                    identity,
                                });
                            }
                            // Only copies get the restore-the-original undo
                            // entry; a replaced move keeps the move-back
                            // record and leaves the original in Trash.
                            if request.move_sources {
                                return;
                            }
                            if let Some(target) = &replaced_target {
                                emit(OperationEvent::Merged {
                                    request_id: request.id,
                                    source: item.source.clone(),
                                    created: Vec::new(),
                                    overwritten: vec![target.clone()],
                                    placed: placed
                                        .map(|placed| HashMap::from([(target.clone(), placed)]))
                                        .unwrap_or_default(),
                                });
                            }
                        },
                    )
                    .await
                    .map(|owed| (target, owed))
                } else {
                    if taken == Taken::Fail
                        && case_variants::taken_by(&target, variants.earlier(index)).await
                    {
                        taken = Taken::Number;
                        aliased = true;
                    }
                    if request.move_sources {
                        move_owing(
                            source.clone(),
                            target.clone(),
                            taken,
                            operation_cancellable.clone(),
                            copying.clone(),
                        )
                        .await
                        .map(|moved| (moved.target, moved.owed))
                    } else {
                        copy_new_with_ownership(
                            source.clone(),
                            target.clone(),
                            taken,
                            operation_cancellable.clone(),
                            Some(progress.clone()),
                            names.clone(),
                            None,
                        )
                        .await
                        .map(|(target, ownership)| {
                            copied_ownership = ownership;
                            (target, Owed::default())
                        })
                    }
                };
                if let Err(error) = &result
                    && let Some(kind) = unheld(error)
                {
                    progress.report(left_out(
                        &item.source,
                        &destination,
                        kind,
                        request.move_sources,
                    ));
                    progress.finish_item(
                        progress.reported().since(started),
                        item_sizes[index],
                        item_files[index],
                        None,
                    );
                    continue;
                }
                let (target, owed) = match result {
                    Ok(result) => result,
                    Err(error) => {
                        settling.finish().await;
                        let (failure, skip_flush) =
                            copy_failure_on_fat32(&error, max_file_size.is_some());
                        let untouched = undone && was_cancelled(&error);
                        let mut failed = settling.failed();
                        if !untouched {
                            failed.push(item.source.clone());
                        }
                        stop_transfer(
                            &written_paths,
                            &emit,
                            request.id,
                            TransferStop {
                                completed: settling.completed(),
                                failed,
                                not_attempted: request.items[index + usize::from(!untouched)..]
                                    .iter()
                                    .map(|item| item.source.clone())
                                    .collect(),
                                affected_locations,
                                failure: joined_failures(failure, settling.failure()),
                                skip_flush,
                            },
                        )
                        .await;
                        return;
                    }
                };
                let target_location = location_for_file(&target);
                if let Some(target) = target_location.clone() {
                    affected_locations.insert(target);
                }
                variants.saved(index, &target).await;
                let cause = planned[index]
                    .as_ref()
                    .and_then(|planned| planned.cause)
                    .or(aliased.then_some(RenameCause::Alias));
                if let (Some(cause), Some(name)) = (cause, transfer_source_name(&sources[index])) {
                    // Keep Both's numbers go unreported; the engine reports a
                    // name the mount refused from the planned one.
                    let saved_as = match (taken, &planned[index]) {
                        (Taken::Fail, Some(planned)) => destination.child(&planned.target),
                        _ => target.clone(),
                    };
                    progress.report(TransferReport {
                        renamed: renamed_item(&destination.child(name), &saved_as, cause)
                            .into_iter()
                            .collect(),
                        ..TransferReport::default()
                    });
                }
                // A merge reports its written paths through Merged instead:
                // copy undo trashes recorded locations, and the merged folder
                // held pre-existing contents that must not be trashed.
                // Undoing a copy deletes it, which must never remove the only
                // copy left after an original was replaced without Trash.
                let created_location = target_location.clone().filter(|_| {
                    !request.move_sources
                        && item.conflict != TransferConflict::Merge
                        && !replaced_permanently.get()
                });
                if records_copies
                    && item.conflict != TransferConflict::ReplaceExisting
                    && let Some(location) = &created_location
                    && let Some(path) = location.native_path()
                    && let Some(ownership) = copied_ownership
                    && let Some(placed) = copy_engine::placed_proven(
                        path.to_path_buf(),
                        ownership,
                        copying.placed_entries.clone(),
                        &operation_cancellable,
                    )
                    .await
                {
                    emit(OperationEvent::Merged {
                        request_id: request.id,
                        source: item.source.clone(),
                        created: Vec::new(),
                        overwritten: Vec::new(),
                        placed: HashMap::from([(location.clone(), placed)]),
                    });
                }
                let done = Done {
                    index,
                    moved_to: target_location,
                };
                let reported = progress.reported().since(started);
                progress.finish_item(
                    reported,
                    item_sizes[index],
                    item_files[index],
                    created_location,
                );
                settling.owe(done, owed, reported.bytes, last).await;
            }
            settling.finish().await;
            if let Some(failure) = settling.failure() {
                stop_transfer(
                    &written_paths,
                    &emit,
                    request.id,
                    TransferStop {
                        completed: settling.completed(),
                        failed: settling.failed(),
                        not_attempted: Vec::new(),
                        affected_locations,
                        failure: Some(failure),
                        skip_flush: false,
                    },
                )
                .await;
                return;
            }
            let completed = settling.completed();
            if !flush_removable_writes(
                written_paths,
                &operation_cancellable,
                &emit,
                request.id,
                &completed,
                affected_locations,
            )
            .await
            {
                return;
            }
            emit(OperationEvent::Pasted {
                request_id: request.id,
                locations: completed,
            });
        });
        operation_handle(cancellable)
    }

    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let mut affected_locations = HashSet::new();
            for item in &request.items {
                for location in [&item.record.current, &item.record.original] {
                    affected_locations.insert(location.clone());
                    if let Some(parent) = location.parent() {
                        affected_locations.insert(parent);
                    }
                }
            }
            for location in &request.cleanup_locations {
                affected_locations.insert(location.clone());
                if let Some(parent) = location.parent() {
                    affected_locations.insert(parent);
                }
            }
            let moved = release::natives(request.items.iter().map(|item| &item.record.current));
            let operation = ActiveOperation::begin(release::parents(&moved).into_iter().chain(
                release::parents(&release::natives(
                    request.items.iter().map(|item| &item.record.original),
                )),
            ));
            let sources = request
                .items
                .iter()
                .map(|item| gio_file_for_location(&item.record.current))
                .collect::<Vec<_>>();
            let TransferSizes {
                sizes: item_sizes,
                total: total_bytes,
                files: item_files,
                total_files,
                ..
            } = match transfer_sizes(&sources, &operation_cancellable, None).await {
                Ok(sizes) => sizes,
                Err(error) if was_cancelled(&error) => {
                    emit(cancelled_event(
                        request.id,
                        Vec::new(),
                        Vec::new(),
                        request
                            .items
                            .iter()
                            .map(|item| item.record.current.clone())
                            .collect(),
                        affected_locations,
                    ));
                    return;
                }
                Err(error) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: Vec::new(),
                        message: crate::services::gio_error_message(&error),
                    });
                    return;
                }
            };
            let replaced = release::natives(
                request
                    .items
                    .iter()
                    .filter(|item| item.conflict == TransferConflict::ReplaceExisting)
                    .map(|item| &item.record.original),
            );
            let cleaned = release::natives(&request.cleanup_locations);
            operation
                .release(
                    moved.into_iter().chain(replaced).chain(cleaned).collect(),
                    &operation_cancellable,
                )
                .await;
            let names = NamePolicy::new(request.names);
            let progress =
                TransferProgressTracker::new(request.id, total_bytes, total_files, emit.clone());
            progress.emit();
            let undone = request
                .items
                .iter()
                .map(|item| owed::Source {
                    location: item.record.current.clone(),
                    merged: false,
                })
                .collect();
            let mut settling =
                Settling::new(request.id, (undone, true), emit.clone(), progress.clone());
            let mut restored_paths = Vec::new();
            for (index, item) in request.items.iter().enumerate() {
                let remaining = || {
                    request.items[index..]
                        .iter()
                        .map(|item| item.record.current.clone())
                        .collect::<Vec<_>>()
                };
                if operation_cancellable.is_cancelled() || settling.failure().is_some() {
                    settling.finish().await;
                    stop_transfer(
                        &restored_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed: settling.completed(),
                            failed: settling.failed(),
                            not_attempted: remaining(),
                            affected_locations,
                            failure: settling.failure(),
                            skip_flush: false,
                        },
                    )
                    .await;
                    return;
                }
                let source = sources[index].clone();
                let started = progress.reported();
                let restored_path = item.record.original.native_path().map(Path::to_path_buf);
                if let Some(path) = restored_path {
                    restored_paths.push(path);
                }
                let target = gio_file_for_location(&item.record.original);
                let undone = undone_when_cancelled(&source, &target);
                let copying = Copying {
                    progress: Some(progress.clone()),
                    names: names.clone(),
                    ..Copying::default()
                };
                let result = if item.conflict == TransferConflict::ReplaceExisting {
                    replace_owing(
                        source,
                        target,
                        true,
                        &operation_cancellable,
                        Some(&mut affected_locations),
                        copying,
                        &|original, _| {
                            if let ReplacedOriginal::Trashed(Some(identity)) = original {
                                emit(OperationEvent::ItemTrashed {
                                    request_id: request.id,
                                    location: item.record.original.clone(),
                                    identity,
                                });
                            }
                        },
                    )
                    .await
                } else {
                    move_owing(
                        source,
                        target,
                        Taken::Fail,
                        operation_cancellable.clone(),
                        copying,
                    )
                    .await
                    .map(|moved| moved.owed)
                };
                let mut owed = match result {
                    Ok(owed) => owed,
                    Err(error) => {
                        settling.finish().await;
                        let failure = (!was_cancelled(&error))
                            .then(|| crate::services::gio_error_message(&error));
                        let untouched = undone && was_cancelled(&error);
                        let mut failed = settling.failed();
                        if !untouched {
                            failed.push(item.record.current.clone());
                        }
                        stop_transfer(
                            &restored_paths,
                            &emit,
                            request.id,
                            TransferStop {
                                completed: settling.completed(),
                                failed,
                                not_attempted: request.items[index + usize::from(!untouched)..]
                                    .iter()
                                    .map(|item| item.record.current.clone())
                                    .collect(),
                                affected_locations,
                                failure: joined_failures(failure, settling.failure()),
                                skip_flush: false,
                            },
                        )
                        .await;
                        return;
                    }
                };
                owed.follow_renames(&restore_renamed(&item.record, &progress).await);
                let reported = progress.reported().since(started);
                progress.finish_item(reported, item_sizes[index], item_files[index], None);
                let done = Done {
                    index,
                    moved_to: Some(item.record.original.clone()),
                };
                let last = index + 1 == request.items.len();
                settling.owe(done, owed, reported.bytes, last).await;
            }
            settling.finish().await;
            if let Some(failure) = settling.failure() {
                stop_transfer(
                    &restored_paths,
                    &emit,
                    request.id,
                    TransferStop {
                        completed: settling.completed(),
                        failed: settling.failed(),
                        not_attempted: Vec::new(),
                        affected_locations,
                        failure: Some(failure),
                        skip_flush: false,
                    },
                )
                .await;
                return;
            }
            let completed = settling.completed();
            if !flush_removable_writes(
                restored_paths,
                &operation_cancellable,
                &emit,
                request.id,
                &completed,
                affected_locations.clone(),
            )
            .await
            {
                return;
            }
            for location in &request.cleanup_locations {
                let file = gio_file_for_location(location);
                let identity = match trash_with_cleanup(&file, &operation_cancellable).await {
                    Ok(identity) => identity,
                    Err(error) if error.is_unsupported() => {
                        match remove_empty_directory(&file, &operation_cancellable).await {
                            Err(error) if was_cancelled(&error) => {
                                emit(cancelled_event(
                                    request.id,
                                    completed,
                                    Vec::new(),
                                    vec![location.clone()],
                                    affected_locations,
                                ));
                                return;
                            }
                            _ => continue,
                        }
                    }
                    Err(error) => {
                        if was_cancelled(&error.error) {
                            emit(cancelled_event(
                                request.id,
                                completed,
                                Vec::new(),
                                vec![location.clone()],
                                affected_locations,
                            ));
                        } else {
                            emit(OperationEvent::TransferFailed {
                                request_id: request.id,
                                completed_locations: completed,
                                message: crate::services::gio_error_message(&error.error),
                            });
                        }
                        return;
                    }
                };
                if let Some(identity) = identity {
                    emit(OperationEvent::ItemTrashed {
                        request_id: request.id,
                        location: location.clone(),
                        identity,
                    });
                }
            }
            emit(OperationEvent::Pasted {
                request_id: request.id,
                locations: completed,
            });
        });
        operation_handle(cancellable)
    }

    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let current = request.current.clone();
            let original = request.original.clone();
            let mut affected_locations = HashSet::new();
            for location in [&current, &original] {
                affected_locations.insert(location.clone());
                if let Some(parent) = location.parent() {
                    affected_locations.insert(parent);
                }
            }
            let roots = release::natives([&current]);
            let operation =
                ActiveOperation::begin(release::parents(&release::natives([&current, &original])));
            operation.release(roots, &operation_cancellable).await;
            let result = if operation_cancellable.is_cancelled() {
                Err(cancelled_local_operation())
            } else if let (Some(source), Some(target)) =
                (current.native_path(), original.native_path())
            {
                rename_native(
                    source.to_path_buf(),
                    target.to_path_buf(),
                    &operation_cancellable,
                )
                .await
            } else {
                move_file_with_progress(
                    gio_file_for_location(&current),
                    gio_file_for_location(&original),
                    gio::FileCopyFlags::NOFOLLOW_SYMLINKS
                        | gio::FileCopyFlags::ALL_METADATA
                        | gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE,
                    operation_cancellable,
                    None,
                )
                .await
            };
            match result {
                Ok(()) => emit(OperationEvent::Renamed {
                    request_id: request.id,
                }),
                Err(error) if was_cancelled(&error) => emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    Vec::new(),
                    vec![current],
                    affected_locations,
                )),
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: crate::services::gio_error_message(&error),
                    password_failure: None,
                }),
            }
        });
        operation_handle(cancellable)
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let roots: Vec<PathBuf> = request
                .entries
                .iter()
                .filter_map(|entry| entry.local_thumbnail_path().map(Path::to_path_buf))
                .collect();
            let operation = ActiveOperation::begin(release::parents(&roots));
            operation.release(roots, &operation_cancellable).await;
            let targets = request
                .entries
                .iter()
                .map(DeletionTarget::from_entry)
                .collect();
            run_deletion(
                request.id,
                targets,
                request.permanent,
                emit,
                operation_cancellable,
            )
            .await;
        });
        operation_handle(cancellable)
    }

    fn undo_copy(&self, request: UndoCopyRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let roots = release::natives(&request.locations);
            let operation = ActiveOperation::begin(release::parents(&roots));
            operation.release(roots, &operation_cancellable).await;
            let mut targets = Vec::with_capacity(request.locations.len());
            for location in &request.locations {
                targets.push(DeletionTarget::probed(location, &operation_cancellable).await);
            }
            run_deletion(request.id, targets, false, emit, operation_cancellable).await;
        });
        operation_handle(cancellable)
    }

    fn undo_merge(
        &self,
        request: UndoMergeRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let roots = release::natives(request.created.iter().chain(&request.overwritten));
            let operation = ActiveOperation::begin(release::parents(&roots));
            operation.release(roots, &operation_cancellable).await;
            run_merge_undo(
                request.id,
                (request.created, request.overwritten, request.placed),
                emit,
                operation_cancellable,
                Rc::new(move |location, cancellable| {
                    trashed_merge_original(location, cancellable, request.originals.clone())
                }),
            )
            .await;
        });
        operation_handle(cancellable)
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let entries = match request.source {
                RestoreSource::TrashEntries(items) => items
                    .into_iter()
                    .map(|item| {
                        Ok(RestoreEntry {
                            source: item.entry.location,
                            display_name: item.entry.display_name,
                            original_target: None,
                            trash_info: None,
                            confirmed_destination: Some(item.destination),
                            physical_path: item.entry.thumbnail_path,
                        })
                    })
                    .collect(),
                RestoreSource::OriginalLocations {
                    locations,
                    originals,
                } => match trashed_entries_for_originals(
                    &locations,
                    &operation_cancellable,
                    &originals,
                )
                .await
                {
                    Ok(entries) => entries,
                    Err(error) if was_cancelled(&error) => {
                        emit(cancelled_event(
                            request.id,
                            Vec::new(),
                            Vec::new(),
                            locations,
                            HashSet::from([Location::uri("trash:///")]),
                        ));
                        return;
                    }
                    Err(error) => {
                        emit(OperationEvent::Failed {
                            request_id: request.id,
                            message: rust_i18n::t!(
                                "Unable to find items in Trash: %{error}",
                                error = error
                            )
                            .into_owned(),
                            password_failure: None,
                        });
                        return;
                    }
                },
            };
            let found = entries.iter().flatten().cloned().collect::<Vec<_>>();
            let _operation = restore::release_trashed(&found, &operation_cancellable).await;
            let total = entries.len();
            let mut errors = Vec::new();
            let mut restored_locations = Vec::new();
            let mut restored = Vec::new();
            let mut failed_locations = Vec::new();
            let mut affected_locations = HashSet::from([Location::uri("trash:///")]);
            let context = RestoreContext::current().await;
            let unlinks = Arc::new(copy_engine::Unlinks::default());
            let worker_count = restore_worker_count(&found).await;
            let mut completed = 0;
            let mut cut_off = Vec::new();
            let source = |entry: &Result<RestoreEntry, (Location, glib::Error)>| match entry {
                Ok(entry) => entry.source.clone(),
                Err((original, _)) => original.clone(),
            };
            for chunk in entries.chunks(worker_count) {
                if operation_cancellable.is_cancelled() {
                    emit(cancelled_event(
                        request.id,
                        restored,
                        failed_locations,
                        entries[completed..].iter().map(source).collect(),
                        affected_locations,
                    ));
                    return;
                }
                let main_context = glib::MainContext::default();
                let tasks = chunk
                    .iter()
                    .cloned()
                    .map(|entry| {
                        let (context, unlinks) = (context.clone(), unlinks.clone());
                        let cancellable = operation_cancellable.clone();
                        main_context.spawn_local(async move {
                            let mut affected = HashSet::new();
                            let result = match &entry {
                                Ok(entry) => restore_trash_entry(
                                    entry,
                                    &context,
                                    &mut affected,
                                    &unlinks,
                                    &cancellable,
                                )
                                .await
                                .map(|(report, identity)| {
                                    (report, identity, restored_destination(entry))
                                }),
                                Err((_, refused)) => Err(refused.clone()),
                            };
                            (entry, result, affected)
                        })
                    })
                    .collect::<Vec<_>>();
                for task in tasks {
                    let (entry, result, affected) = match task.await {
                        Ok(outcome) => outcome,
                        Err(_) => (
                            entries[completed].clone(),
                            Err(io_error("Restore worker failed")),
                            HashSet::new(),
                        ),
                    };
                    let display_name = match &entry {
                        Ok(entry) => entry.display_name.clone(),
                        Err((original, _)) => original.display_name(),
                    };
                    affected_locations.extend(affected);
                    completed += 1;
                    let restored_location = match result {
                        Err(error) if was_cancelled(&error) => {
                            cut_off.push(source(&entry));
                            None
                        }
                        Err(error) => {
                            errors.push(item_error(
                                &display_name,
                                crate::services::gio_error_detail(&error),
                            ));
                            failed_locations.push(source(&entry));
                            None
                        }
                        Ok((report, identity, destination)) => {
                            if !report.is_empty() {
                                emit(OperationEvent::ItemsReported {
                                    request_id: request.id,
                                    report,
                                });
                            }
                            restored_locations.push(source(&entry));
                            emit(OperationEvent::ItemRestored {
                                request_id: request.id,
                                location: destination.clone(),
                                identity,
                            });
                            restored.push(destination.clone());
                            Some(destination)
                        }
                    };
                    emit(OperationEvent::RestoreProgress {
                        request_id: request.id,
                        completed,
                        total,
                        restored_location,
                    });
                }
                if !cut_off.is_empty() || operation_cancellable.is_cancelled() {
                    cut_off.extend(entries[completed..].iter().map(source));
                    emit(cancelled_event(
                        request.id,
                        restored,
                        failed_locations,
                        cut_off,
                        affected_locations,
                    ));
                    return;
                }
            }
            if errors.is_empty() {
                emit(OperationEvent::Restored {
                    request_id: request.id,
                    locations: restored_locations,
                    restored,
                });
            } else {
                emit(OperationEvent::RestoreCompletedWithErrors {
                    request_id: request.id,
                    restored_locations,
                    restored,
                    message: operation_error_summary(&errors, FailedAction::Restore),
                });
            }
        });
        operation_handle(cancellable)
    }

    fn recover(&self, request: RecoverRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        leftovers::start_recovery(request, emit)
    }

    fn sweep_probes(&self, folder: &Location, names: &[OsString]) {
        leftovers::start_sweep(folder, names);
    }

    fn review_leftovers(
        &self,
        folder: &Location,
        names: Vec<OsString>,
        done: Box<dyn FnOnce(Vec<crate::services::ReviewedLeftover>)>,
    ) -> LoadHandle {
        leftovers::start_review(folder, names, done)
    }

    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        archive::compress(request, emit)
    }

    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        archive::extract(request, emit)
    }
}
