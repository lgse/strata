// SPDX-License-Identifier: MIT

use super::{
    fs_ops::{Entry, OffMain, fs},
    holders,
    placement::{PlaceError, RenameFailure},
    release::{self, ActiveOperation},
};
use crate::{
    adapters::{
        gio_file_for_location,
        local_files::trash_thumbnail_path,
        location_for_file,
        trash_restore::{
            RestoreContext, absolute_original, candidate_trash_roots, decode_trashinfo_path,
            plan_restore_for_location,
        },
        trash_support::trash_with_cleanup,
    },
    model::Location,
    services::{
        ItemIdentity, KeptCause, KeptItem, MoveRecord, OperationEvent, OperationRequestId,
        PlacedSnapshot, Proof, RenameCause, RenamedItem, TransferReport,
    },
};
use gio::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    future::Future,
    io,
    os::{
        fd::{AsFd, OwnedFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::Arc,
};

use super::{
    await_cancellable, cancelled_event, cancelled_local_operation, copy_engine,
    deletion::local_parent_worker_count,
    deletion_error_summary, deletion_undone_when_cancelled, io_error, item_error, kept_items,
    mount_profile::identity_at,
    native_fs::{
        identity_at_path, open_local_parent_beneath, open_local_parent_directory, os_error_detail,
        trashed_identity,
    },
};

use super::{
    owed, placement, placement_failure, progress::TransferProgressTracker, rename_failure_error,
    run_blocking, run_local_fs_step, set_display_name, translated_io_error,
};

use super::{gio_names::next_children, was_cancelled};

mod lookup;
mod merge_undo;
mod moves;
mod names;

pub(super) use lookup::*;
pub(super) use merge_undo::*;
pub(super) use moves::*;
pub(super) use names::*;

pub(super) async fn release_trashed(
    entries: &[RestoreEntry],
    cancellable: &gio::Cancellable,
) -> ActiveOperation {
    let roots: Vec<PathBuf> = entries
        .iter()
        .filter_map(|entry| {
            entry
                .physical_path
                .clone()
                .or_else(|| entry.source.native_path().map(Path::to_path_buf))
        })
        .collect();
    let destinations: Vec<PathBuf> = entries
        .iter()
        .filter_map(|entry| {
            restored_destination(entry)
                .native_path()
                .map(Path::to_path_buf)
        })
        .collect();
    let operation = ActiveOperation::begin(release::parents(&destinations));
    operation.release(roots, cancellable).await;
    operation
}

pub(super) fn restored_destination(entry: &RestoreEntry) -> Location {
    entry
        .confirmed_destination
        .as_ref()
        .map(|path| Location::local(path.clone()))
        .or_else(|| entry.original_target.clone())
        .unwrap_or_else(|| entry.source.clone())
}

pub(super) async fn restore_worker_count(entries: &[RestoreEntry]) -> usize {
    local_parent_worker_count(entries.iter().filter_map(|entry| {
        restored_destination(entry)
            .native_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }))
    .await
}

pub(super) fn trashed_merge_original(
    location: Location,
    cancellable: gio::Cancellable,
    originals: HashMap<Location, ItemIdentity>,
) -> Pin<Box<dyn Future<Output = Result<RestoreEntry, glib::Error>>>> {
    Box::pin(async move {
        let entries = trashed_entries_for_originals(
            std::slice::from_ref(&location),
            &cancellable,
            &originals,
        )
        .await?;
        match entries.into_iter().next() {
            Some(found) => found.map_err(|(_, error)| error),
            None => Err(glib::Error::new(
                gio::IOErrorEnum::NotFound,
                &crate::i18n::tr("The original is no longer in Trash"),
            )),
        }
    })
}
