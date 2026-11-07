// SPDX-License-Identifier: MIT

//! Emulates filesystems that lack a primitive. A scope rooted at a temporary
//! directory applies its quirks to every seam call whose directory lies
//! inside it; other calls reach the system unchanged. Separate scopes act as
//! separate mounts.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    ffi::{OsStr, OsString},
    ops::ControlFlow,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd},
        unix::ffi::OsStringExt,
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use gtk::{gio, glib, prelude::FileExt};
use rustix::{
    fs::{Access, AtFlags, FileType, FsWord, Mode, OFlags, RenameFlags, ResolveFlags, StatxFlags},
    io::Errno,
};

use super::super::copy_engine::clock_for_test;
use super::super::environment::FsEnvironment;
use super::super::fs_ops::{
    Entry, EntryStat, FolderStamp, FsOps, FsStat, Listed, System, Watched, assert_off_main_thread,
};
use super::super::holders;
use super::super::native_fs::LocalFileIdentity;
use crate::{
    adapters::volume::{
        Family, MountKey, MountSite, MountTable, MountType, family, forget_test_mount_capabilities,
    },
    services::Holder,
};

mod environment;
mod kinds;
mod lookup;
mod mounts;
mod names;
mod naming;
mod opens;
mod ops;
mod presets;
mod renames;
mod report;
mod scope;
mod silly;

use self::{environment::*, lookup::*, mounts::*, names::*, presets::*, scope::*, silly::*};
pub(crate) use environment::{SCOPED, Scoped, smb_base};
pub(crate) use kinds::{
    Aside, Births, CachedListing, CachedTimes, Exchange, NameOpens, Op, OverEmptyDirectory, Preset,
    Quirks, Racer, Silly, Support, TmpfileLink, Trash,
};
pub(crate) use lookup::{
    autodisable_serverino, evict, gio_device, hold, recreated, renumbered,
    skips_malformed_name_on_fuse,
};
pub(crate) use scope::{FsQuirks, HoldGuard};

#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub op: Op,
    pub names: Vec<OsString>,
    /// A stat that forced revalidation.
    pub forced: bool,
}

#[derive(Clone, Copy)]
struct WrittenTime {
    birth: Option<(i64, u32)>,
    modified: (i64, u32),
}

#[derive(Clone, Copy)]
struct Seen {
    modified: (i64, u32),
    changed: (i64, u32),
    fetch: Fetch,
}

#[derive(Clone, Copy)]
struct Fetch {
    at: SystemTime,
    own_changes: u64,
}

impl Fetch {
    fn kept(&self, kept_for: Duration, now: Fetch) -> bool {
        self.own_changes == now.own_changes
            && now
                .at
                .duration_since(self.at)
                .is_ok_and(|age| age < kept_for)
    }
}

struct Scope {
    id: u64,
    root: PathBuf,
    mount_id: Option<u64>,
    host: Option<MountType>,
    own_device: bool,
    state: Mutex<State>,
    /// Makes looking a name up and creating it one step, as a filesystem that
    /// compares names itself does.
    naming: Mutex<()>,
}

struct State {
    quirks: Quirks,
    calls: Vec<Call>,
    inodes: HashMap<PathBuf, u64>,
    births: HashMap<PathBuf, i64>,
    /// The birth time of each folder a rename reset, by device and inode.
    reset_births: HashMap<(u64, u64), (i64, u32)>,
    /// Real and reported inode number per name a hard link added, or a lost
    /// rename reply left reporting its old number.
    link_names: HashMap<PathBuf, (u64, u64)>,
    links_made: u64,
    stale: HashMap<PathBuf, EntryStat>,
    written: HashSet<(u64, u64)>,
    written_times: HashMap<(u64, u64), WrittenTime>,
    renumbered: HashSet<PathBuf>,
    /// Writebacks lost so far, and how many each open folder has reported.
    writebacks_lost: u64,
    reported_losses: HashMap<RawFd, u64>,
    /// Each name a file was opened by, by device and inode, with the
    /// descriptor that opened it.
    opened_names: Vec<((u64, u64), PathBuf, RawFd)>,
    /// Folders made read-only that no revalidating stat has seen since.
    read_only_unseen: HashSet<PathBuf>,
    /// The change time each entry's last stat saw, and when, by device and inode.
    seen_change_times: HashMap<(u64, u64), ((i64, u32), SystemTime)>,
    seen_times: HashMap<(u64, u64), Seen>,
    listings: HashMap<PathBuf, (Listed, Fetch)>,
    own_changes: u64,
    unplugged: bool,
    serverino_disabled: bool,
    cached: HashMap<PathBuf, (u64, Option<(i64, u32)>)>,
    created: HashSet<(u64, u64)>,
    made_up: HashMap<(u64, u64), (i64, u32)>,
    recorded_births: HashMap<(u64, u64), SystemTime>,
    latest_birth: SystemTime,
    holds: HashMap<(u64, u64), usize>,
    held_paths: HashMap<(u64, u64), PathBuf>,
    closed_holds: Vec<(PathBuf, SystemTime)>,
    leftovers: Vec<SillyLeftover>,
    silly_renames: u64,
}

struct SillyLeftover {
    folder: PathBuf,
    folder_key: (u64, u64),
    name: Option<OsString>,
    key: (u64, u64),
    unheld_since: Option<SystemTime>,
}

impl State {
    fn fetch(&self) -> Fetch {
        Fetch {
            at: clock_for_test(),
            own_changes: self.own_changes,
        }
    }

    fn recorded_birth(&mut self, key: (u64, u64)) -> Option<(i64, u32)> {
        let latest = &mut self.latest_birth;
        let born = *self.recorded_births.entry(key).or_insert_with(|| {
            *latest = SystemTime::now().max(*latest + Duration::from_nanos(1));
            *latest
        });
        timespec(born)
    }
}

static SCOPES: Mutex<Vec<Arc<Scope>>> = Mutex::new(Vec::new());
static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
static FIRST_FOLDER_BIRTHS: Mutex<BTreeMap<(u64, u64), (i64, u32)>> = Mutex::new(BTreeMap::new());

fn scopes() -> MutexGuard<'static, Vec<Arc<Scope>>> {
    SCOPES.lock().unwrap_or_else(|error| error.into_inner())
}

fn first_folder_births() -> MutexGuard<'static, BTreeMap<(u64, u64), (i64, u32)>> {
    FIRST_FOLDER_BIRTHS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}
