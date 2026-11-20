// SPDX-License-Identifier: MIT

//! Copies a folder's tree on the shared work queue. Entries are made
//! exclusively, so in a folder this copy made a taken name is a case alias,
//! someone else's entry, or the copy's own entry whose reply was lost.

use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use rustix::{
    fs::{AtFlags, FileType, Mode},
    io::Errno,
};

use super::{
    Barrier, Copy, Created, Failure, Mount, Opened, Placing, Source, Written, best_effort,
    creation_failure,
    data::Data,
    direct_folder, from_stage, is_fatal, is_unsupported,
    links::{self, Links, Role},
    lock,
    manifest::{self, Pending},
    metadata::Snapshot,
    open_failure, open_file, open_one_name, rebase,
    restrictive::{self, Restrictive},
    retry_refused, stage, stage_left,
};
use crate::{
    adapters::{
        local_operations::{
            cancelled_local_operation,
            fs_ops::{Entry, EntryStat, FsOps},
            io_error,
            leftovers::{self, Refused},
            mount_profile::{FolderRules, Site, crosses},
            names::{Key, PlanEntry, Planned, numbered_names, plan_names},
            native_fs::{LocalFileIdentity, compare, os_error_detail},
            placement::{self, PlaceError},
            replacement::{StageContent, StagedSibling},
            translated_io_error,
            work_queue::{self, Group, Job, WorkQueue},
        },
        volume::{Family, MountKey, NameProfile},
    },
    services::{IdentityTraits, KeptCause, Proof, RenameCause, SpecialKind},
};

mod file;
mod folder;
mod merged;
mod names;
mod nodes;
mod place;

pub(super) use file::File;
use file::Finish;
pub(super) use folder::Folder;
use folder::{Made, Making, file_name, target_children};
pub(super) use merged::Merged;
use names::planned_rename;
pub(super) use names::{Names, Naming, make_numbered};
pub(super) use nodes::{adopt, copy_fifo, copy_link, finish_pipe, made_again, read_link};
use nodes::{claim_file, claim_folder, fresh_file};
pub(in crate::adapters::local_operations) use nodes::{refuses_node, taken};
pub(super) use place::Place;

/// `FS_CASEFOLD_FL`, which folders made inside a casefolded one inherit.
const CASEFOLD: u32 = 0x4000_0000;

type Queue = WorkQueue<TreeJob>;
type Completion = Option<Arc<Group<TreeJob>>>;

pub(super) struct Tree {
    copy: Arc<Copy>,
    workers: usize,
    restrictive: Mutex<Vec<(usize, Restrictive)>>,
    links: Links,
    scopes: AtomicUsize,
    created: Option<Arc<Created>>,
    stages: Mutex<Vec<StagedSibling>>,
    placed: Mutex<Vec<PathBuf>>,
    copies: Mutex<Vec<manifest::Published>>,
    nested: Mutex<Nested>,
    source: OnceLock<(Mount, Site)>,
}

#[derive(Default)]
struct Nested {
    root: Option<MountKey>,
    barriers: Vec<Barrier>,
}

impl Tree {
    /// Where a mount is probed for links, outside the copy.
    pub(super) fn probe_links_in(&self, mount: &Mount, dir: Arc<OwnedFd>) {
        self.links.probe_in(mount.key, dir);
    }

    fn made(&self, path: &Path, identity: LocalFileIdentity) {
        if let Some(created) = &self.created {
            created.add(path.to_path_buf(), identity);
        }
    }

    fn take_restrictive(&self, scope: usize) -> Vec<Restrictive> {
        let mut recorded = lock(&self.restrictive);
        let (taken, kept) = std::mem::take(&mut *recorded)
            .into_iter()
            .partition::<Vec<_>, _>(|(recorded, _)| *recorded == scope);
        *recorded = kept;
        taken.into_iter().map(|(_, folder)| folder).collect()
    }

    fn reach(&self, mount: &Mount, dir: BorrowedFd<'_>, path: &Path) -> Result<(), Failure> {
        let fs = self.copy.fs;
        let mut nested = lock(&self.nested);
        let root = *nested.root.get_or_insert(mount.key);
        if mount.key != root
            && nested
                .barriers
                .iter()
                .all(|barrier| barrier.key != mount.key)
        {
            nested
                .barriers
                .push(Barrier::at(fs, dir, path.to_path_buf())?);
        }
        Ok(())
    }

    fn source_in(&self, fs: &dyn FsOps, folder: BorrowedFd<'_>) -> &(Mount, Site) {
        self.source.get_or_init(|| {
            let mount = Mount::new(fs, folder);
            (mount, Site::of(mount.key, mount.names, folder))
        })
    }

    fn opens_one_name_per_file(&self) -> bool {
        self.source.get().map(|(mount, _)| mount.family) == Some(Family::Smb)
    }

    /// The tree's source is read through one mount and site, which in practice hold every source
    /// folder on its device.
    fn crosses(&self, source: &EntryStat, target: &EntryStat, site: &Site) -> bool {
        self.source.get().is_none_or(|(_, source_site)| {
            crosses(
                (source.identity.device, source_site),
                (target.identity.device, site),
            )
        })
    }

    fn plan(
        &self,
        source: BorrowedFd<'_>,
        rules: &FolderRules,
        planning: (bool, Key),
    ) -> Result<Vec<(OsString, FileType, Planned)>, Failure> {
        target_children(self.copy.fs, source, &rules.fixed, planning)
    }

    fn settle_stage(&self, identity: LocalFileIdentity) {
        lock(&self.stages).retain(|staged| staged.identity != identity);
    }
}

pub(super) enum TreeJob {
    Folder(Arc<Tree>, Folder),
    File(Arc<Tree>, File),
    Finish(Arc<Tree>, Finish),
    Merge(Arc<Tree>, Merged),
    Place(Arc<Tree>, Place),
}

impl Job for TreeJob {
    type Error = glib::Error;

    fn run(self, queue: &Queue) {
        if queue.is_stopped() {
            return;
        }
        match self {
            Self::Folder(tree, folder) => folder.run(&tree, queue),
            Self::File(tree, file) => file.run(&tree, queue),
            Self::Finish(tree, finish) => finish.run(&tree, queue),
            Self::Merge(tree, merged) => merged.run(&tree, queue),
            Self::Place(tree, place) => place.run(&tree, queue),
        }
    }

    fn cancelled() -> glib::Error {
        cancelled_local_operation()
    }

    fn panicked() -> glib::Error {
        translated_io_error("The copy stopped unexpectedly")
    }

    fn not_started(error: io::Error) -> glib::Error {
        io_error(rust_i18n::t!(
            "Could not start the copy: %{error}",
            error = error
        ))
    }
}

pub(super) struct Walked {
    pub(super) restrictive: Vec<Restrictive>,
    pub(super) links: Links,
    pub(super) stages: Vec<StagedSibling>,
    pub(super) placed: Vec<PathBuf>,
    pub(super) copies: Vec<manifest::Published>,
    pub(super) nested: Vec<Barrier>,
}

/// Every descriptor the walk opened is closed when it returns.
pub(super) fn walk(
    copy: &Arc<Copy>,
    workers: usize,
    created: Option<Arc<Created>>,
    root: impl FnOnce(Arc<Tree>) -> TreeJob,
) -> (Result<(), glib::Error>, Walked) {
    let tree = Arc::new(Tree {
        copy: copy.clone(),
        workers: workers.max(1),
        restrictive: Mutex::default(),
        links: Links::default(),
        scopes: AtomicUsize::new(0),
        created,
        stages: Mutex::default(),
        placed: Mutex::default(),
        copies: Mutex::default(),
        nested: Mutex::default(),
        source: OnceLock::new(),
    });
    let queue = Arc::new(Queue::new(copy.cancelled.clone()));
    queue.enqueue(root(tree.clone()));
    work_queue::run_workers(&queue, tree.workers, None, |work| {
        thread::Builder::new().spawn(work)
    });
    let result = queue.result();
    drop(queue);
    let walked = Walked {
        restrictive: std::mem::take(&mut *lock(&tree.restrictive))
            .into_iter()
            .map(|(_, folder)| folder)
            .collect(),
        links: tree.links.take(),
        stages: std::mem::take(&mut *lock(&tree.stages)),
        placed: std::mem::take(&mut *lock(&tree.placed)),
        copies: std::mem::take(&mut *lock(&tree.copies)),
        nested: std::mem::take(&mut lock(&tree.nested).barriers),
    };
    (result, walked)
}
