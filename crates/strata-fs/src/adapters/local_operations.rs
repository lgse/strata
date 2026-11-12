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
use replacement::{Merging, Replacing};
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

mod flush;
pub use flush::flush_filesystem;
pub(crate) use flush::*;
#[cfg(any(test, feature = "test-support"))]
pub use flush::{
    clear_sync_probe, install_sync_probe, release_sync_probe, sync_probe_is_blocked,
    sync_probe_len, sync_probe_observations,
};
mod paste_plan;
pub(crate) use paste_plan::*;
mod gio_names;
use gio_names::*;
mod copying;
use copying::*;
mod moving;
use moving::*;
mod copy_engine;
mod create_entry;
mod deferred_close;
mod paste;
mod rename;
mod restore_items;
mod undo_move;
use deferred_close::WaitCancelled;
mod environment;
mod fs_ops;
#[cfg(any(test, feature = "test-support"))]
pub mod fs_quirks;
mod holders;
mod leftovers;
pub(super) use leftovers::Deadline;
mod merge_aside;
mod mount_profile;
mod owed;
mod place_entry;
mod placement;
pub mod probes;
mod release;
pub use release::ActiveOperation;
mod transfer_targets;
pub(super) mod trash;
mod typed_names;
mod work_queue;

#[cfg(any(test, feature = "test-support"))]
pub use archive::ArchiveListing;
pub use archive::{
    ARCHIVE_UNSUPPORTED_MESSAGE, ArchiveListingStatus, decode_archive_listing,
    list_archive_entries_direct,
};

#[cfg(any(test, feature = "test-support"))]
pub use archive::write_compression_fixture;
pub use fs_ops::Watched;
pub(in crate::adapters) use fs_ops::{
    FolderStamp, folder_stamp, list_folder, stamp_mount, unwatch, watch,
};
#[cfg(any(test, feature = "test-support"))]
pub use fs_quirks::{Births, FsQuirks, Op as FsOp, Quirks, evict};
pub use transfer_targets::{Collision as TransferTargetCollision, transfer_targets};
pub use typed_names::{
    blocked_folder as typed_folder_blocked, refused_in as typed_name_refusal,
    rename_taken as typed_rename_is_taken,
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

use gio::prelude::*;

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
        OperationProvider, OperationRequestId, PasteItem, PasteRequest, PlaceEntryRequest,
        RecoverRequest, RenameCause, RenameRequest, RenamedItem, ReplacedOriginal, RestoreRequest,
        RestoreSource, SkippedItem, SpecialKind, TransferConflict, TransferReport, UndoCopyRequest,
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
        rename::rename(request, emit)
    }

    fn place_entry(
        &self,
        request: PlaceEntryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        place_entry::start(request, emit)
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
        paste::paste(request, emit)
    }

    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        undo_move::undo_move(request, emit)
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
        restore_items::restore(request, emit)
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
