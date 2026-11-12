// SPDX-License-Identifier: MIT

use gio::prelude::FileExt;
use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
};

use super::*;

mod administrator;
mod archive_activation;
mod background_operations;
mod camera_photos;
#[path = "deferred/tests.rs"]
mod deferred;
#[path = "directory_changes/tests.rs"]
mod directory_changes;
mod events;
mod folder_counts;
mod folder_sorts;
mod leftovers;
mod location_input;
mod metadata;
mod monitor;
mod navigation;
#[path = "operation_events/tests.rs"]
mod operation_events;
mod operations;
mod preferences;
mod recent;
mod relocation;
mod reveal;
mod selection;
#[path = "sorting/tests.rs"]
mod staged_sort;
mod staging;
mod undo;
mod undo_refresh;
use crate::{
    model::{EntryKind, MetadataValue},
    services::{
        CancelledOperation, CompressRequest, DirectoryEvent, Evidence, ExtractRequest,
        IdentityTraits, InodeNumbers, LoadHandle, MetadataOutcome, MetadataRequest, MetadataUpdate,
        MountToken, StampAnswer, UndoCopyRequest, UndoMoveRequest, Watching,
    },
};

mod providers;
mod scripted;
mod sources;
mod support;

use providers::*;
use scripted::*;
use sources::*;
use support::*;
