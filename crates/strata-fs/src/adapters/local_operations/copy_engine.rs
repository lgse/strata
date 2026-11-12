// SPDX-License-Identifier: MIT

//! Copies native items through descriptors on worker threads. A copy never
//! replaces an entry, and shows a partial item under its final name only
//! where the destination cannot place a stage.

mod barrier;
mod copy;
mod data;
mod entry_points;
mod failure;
mod file_copy;
mod folders;
mod follow;
mod links;
mod manifest;
mod metadata;
mod mount;
mod new_items;
mod open;
mod placing;
mod restrictive;
pub(super) mod stage;
mod tree;

use std::{
    borrow::Borrow,
    collections::{BTreeSet, HashMap, HashSet},
    ffi::{OsStr, OsString},
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use rustix::{
    fs::{AtFlags, FileType, Mode},
    io::Errno,
};

use super::{
    INVALID_NAME_REASON, Unheld, blocking_join_message, cancelled_local_operation,
    copy_failure_after_cleanup,
    deferred_close::{self, Pause},
    deletion::{
        PinnedEntry, bounded_local_worker_count, local_delete_worker_count,
        parallel_delete_local_blocking_with_workers,
    },
    fs_ops::{Entry, EntryStat, FsOps, OffMain, fs},
    io_error,
    leftovers::{self, Deadline},
    mount_profile::{self, FolderRules, NamePolicy, Site, known_mount},
    names::{Taken, may_be_refused, numbered_names, stricter_names},
    native_fs::{
        LocalFileIdentity, Pinned, compare, open_local_parent_directory, os_error_detail,
        os_error_text,
    },
    placement::{self, ENOTSUPP, PlaceError},
    placement_error_kind, placement_reason,
    progress::{Reported, TransferProgressTracker},
    replacement::{StageContent, StagedSibling, placement_error},
    run_blocking, run_cancellable, translated_io_error,
};
use crate::{
    adapters::volume::{
        CoarseTimes, Family, FuseFormat, MountCapabilities, MountKey, MountNames, NameLimit,
        learn_mount_capability, mount_capabilities,
    },
    model::{Location, hidden_names::HiddenName},
    services::{
        Attributes, IdentityTraits, KeptCause, NameMode, PlacedSnapshot, Proof, RenameCause,
        RenamedItem, SkippedItem, SpecialKind, TransferReport,
    },
};
pub(super) use barrier::Barrier;
use data::Data;
#[cfg(test)]
pub(super) use data::set_kernel_for_test;
#[cfg(any(test, feature = "test-support"))]
pub(super) use manifest::now as clock_for_test;
pub(super) use manifest::{Manifest, Ownership, Published, Unlinks, now, sleep};
use manifest::{Placed, Removed};
#[cfg(test)]
pub(super) use manifest::{set_clock_for_test, set_placed_entries_for_test};
use metadata::Snapshot;
pub(super) use restrictive::Restrictive;
pub(super) use tree::{refuses_node, taken};

#[derive(Default)]
pub(super) struct SharedProgress {
    bytes: AtomicU64,
    files: AtomicUsize,
    current: Mutex<Option<String>>,
}

impl SharedProgress {
    pub(super) fn reported(&self) -> Reported {
        Reported {
            bytes: self.bytes.load(Ordering::Relaxed),
            files: self.files.load(Ordering::Relaxed),
        }
    }

    pub(super) fn take_current(&self) -> Option<String> {
        lock(&self.current).take()
    }

    fn add_bytes(&self, bytes: u64) {
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    fn add_file(&self) {
        self.files.fetch_add(1, Ordering::Relaxed);
    }

    fn start_file(&self, name: &OsStr) {
        *lock(&self.current) = Some(name.to_string_lossy().into_owned());
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

use copy::*;
pub(super) use entry_points::*;
pub(super) use failure::*;
use file_copy::*;
use folders::*;
pub(super) use follow::*;
use mount::*;
use new_items::*;
use open::*;
pub(super) use placing::*;
