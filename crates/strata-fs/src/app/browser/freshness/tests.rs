// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsString,
    time::{Duration, Instant, UNIX_EPOCH},
};

use super::*;
use crate::{
    app::BrowserEvent,
    model::{EntryKind, FileEntry, MetadataValue},
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, Evidence, FileSource, IdentityTraits,
        InodeNumbers, ItemIdentity, LocationValidationError, Watching,
        file_activity::OperationScope,
    },
};

mod checks;
mod coarse_stamps;
mod departures;
mod fixtures;
mod folders;
mod kept_refresh;
mod notices;
mod presence;
mod relists;
mod stalled_mount;

use fixtures::*;
use folders::*;
