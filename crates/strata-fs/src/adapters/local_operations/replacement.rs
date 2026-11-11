// SPDX-License-Identifier: MIT

use super::fs_ops::{Entry, EntryStat, OffMain, fs, list_folder};
use crate::{
    adapters::{
        gio_file_for_location, location_for_file,
        trash_support::{TrashError, trash_with_cleanup},
    },
    model::{Location, hidden_names::HiddenName},
    services::{
        ItemIdentity, KeptCause, KeptItem, PlacedSnapshot, ReplacedOriginal, TransferReport,
    },
};
use gio::prelude::*;

use super::{
    owed::{Deletion, Owed},
    placement::{Displacement, Exchanged, PlaceError, RenameFailure},
};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    future::Future,
    ops::ControlFlow,
    os::fd::{AsFd, OwnedFd},
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{Arc, atomic::AtomicUsize},
};

use super::{
    CopyInto, CopyRecord, GioNames, LeftOut, NamedChildren, TargetFolder, await_cancellable,
    blocking_join_message, cancelled_local_operation, copy_engine, copy_failure_after_cleanup,
    copying::{GioCopy, copy_tree},
    deletion::{
        PinnedEntry, local_delete_worker_count, parallel_delete_local,
        parallel_delete_local_blocking_with_workers, parallel_delete_local_with,
    },
    device, existing_folder_names, io_error, is_trash_unsupported_failure,
    leftovers::Deadline,
    merge_aside,
    mount_profile::{self, NamePolicy, known_mount},
    native_fs::{
        LocalFileIdentity, Pinned, Recorded, open_local_parent_directory, os_error_detail,
    },
    native_path, os_io_error, placement, placement_error_kind, placement_failure, placement_reason,
    progress::TransferProgressTracker,
    release::ActiveOperation,
    run_blocking, translated_io_error, trash, was_cancelled,
};

mod merge;
mod messages;
mod plan;
mod replace;
mod stage;
mod staged;

pub(super) use merge::*;
pub(super) use messages::*;
pub(super) use plan::*;
pub(super) use replace::*;
pub(super) use stage::*;
pub(super) use staged::*;
