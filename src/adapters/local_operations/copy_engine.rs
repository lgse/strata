// SPDX-License-Identifier: MIT

//! Copies native items through descriptors on worker threads. A copy never
//! replaces an entry, and shows a partial item under its final name only
//! where the destination cannot place a stage.

mod barrier;
mod data;
mod links;
mod manifest;
mod metadata;
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

use gtk::{gio, glib};
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
pub(super) use manifest::{Manifest, Ownership, Published, Unlinks, now, sleep};
use manifest::{Placed, Removed};
#[cfg(test)]
pub(super) use manifest::{now as clock_for_test, set_clock_for_test, set_placed_entries_for_test};
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

pub(super) enum Destination {
    New(PathBuf, Taken),
    Merge(PathBuf, Option<Arc<Placing>>),
}

pub(super) struct Placing {
    root: PathBuf,
    wanted: HashSet<PathBuf>,
    unwanted: Option<HashSet<PathBuf>>,
    aside: HashSet<PathBuf>,
    placed: Mutex<Vec<(PathBuf, Placed)>>,
    over_aside: Mutex<HashMap<PathBuf, Ownership>>,
    left: Arc<AtomicUsize>,
}

impl Placing {
    /// Beneath `root`, `wanted` are the paths undo reverts; where the merge records all it
    /// creates, `unwanted` are the overwritten paths undo leaves.
    pub(super) fn new(
        root: PathBuf,
        wanted: HashSet<PathBuf>,
        unwanted: Option<HashSet<PathBuf>>,
        aside: HashSet<PathBuf>,
        left: Arc<AtomicUsize>,
    ) -> Arc<Self> {
        Arc::new(Self {
            root,
            wanted,
            unwanted,
            aside,
            placed: Mutex::default(),
            over_aside: Mutex::default(),
            left,
        })
    }

    fn wants(&self, path: &Path) -> bool {
        self.wanted.contains(path)
    }

    fn placed(
        &self,
        fs: &dyn FsOps,
        (dir, name): (BorrowedFd<'_>, &OsStr),
        path: &Path,
        ownership: &Ownership,
        cancelled: &AtomicBool,
    ) {
        let path = self.root.join(path);
        if self.aside.contains(&path) {
            lock(&self.over_aside).insert(path.clone(), ownership.clone());
        }
        let wanted = match &self.unwanted {
            Some(unwanted) => !unwanted.contains(&path),
            None => self.wants(&path),
        };
        if wanted {
            self.record_proven(fs, (dir, name), path, ownership, cancelled);
        }
    }

    fn record_proven(
        &self,
        fs: &dyn FsOps,
        at: (BorrowedFd<'_>, &OsStr),
        path: PathBuf,
        ownership: &Ownership,
        cancelled: &AtomicBool,
    ) {
        match manifest::placed_proven(fs, at, cancelled, &self.left, ownership) {
            Ok(placed) => lock(&self.placed).push((path, placed)),
            Err(failure) => {
                tracing::warn!(?path, error = %failure.into_error(at.1), "could not record a placed item")
            }
        }
    }

    pub(super) fn over_aside(&self) -> HashMap<PathBuf, Ownership> {
        std::mem::take(&mut *lock(&self.over_aside))
    }

    pub(super) fn take(&self) -> Vec<(PathBuf, PlacedSnapshot)> {
        std::mem::take(&mut *lock(&self.placed))
            .into_iter()
            .map(|(path, placed)| (path, PlacedSnapshot::new(placed)))
            .collect()
    }
}

pub(super) fn placed_budget() -> Arc<AtomicUsize> {
    Arc::new(manifest::placed_entries())
}

#[cfg(test)]
pub(super) async fn placed(
    path: PathBuf,
    left: Arc<AtomicUsize>,
    cancellable: &gio::Cancellable,
) -> Option<PlacedSnapshot> {
    let recorded = run_cancellable(cancellable, move |cancelled| {
        let (parent, name) = split(&path).ok()?;
        placed_within(fs(), (parent.as_fd(), &name), &cancelled, &left)
    })
    .await;
    recorded.ok().flatten()
}

#[cfg(test)]
fn placed_within(
    fs: &dyn FsOps,
    (dir, name): (BorrowedFd<'_>, &OsStr),
    cancelled: &AtomicBool,
    left: &AtomicUsize,
) -> Option<PlacedSnapshot> {
    match manifest::placed(fs, (dir, name), cancelled, left) {
        Ok(placed) => Some(PlacedSnapshot::new(placed)),
        Err(failure) => {
            let error = failure.into_error(name);
            tracing::warn!(?name, %error, "could not record a placed item");
            None
        }
    }
}

pub(super) async fn remove_placed(
    path: PathBuf,
    placed: PlacedSnapshot,
    unlinks: Arc<Unlinks>,
    cancellable: &gio::Cancellable,
) -> Removed {
    let failed = |error| Removed {
        kept: Vec::new(),
        removed: false,
        result: Err(error),
    };
    let removed = run_cancellable(cancellable, move |cancelled| {
        let Some(placed) = placed.get::<Placed>() else {
            return failed(translated_io_error(
                "The record of the placed item is unreadable",
            ));
        };
        let Some(folder) = path.parent() else {
            return failed(io_error(crate::i18n::tr("Invalid local filesystem target")));
        };
        let parent = match open_local_parent_directory(folder) {
            Ok(parent) => Arc::new(parent),
            Err(_)
                if std::fs::symlink_metadata(folder)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Removed {
                    kept: Vec::new(),
                    removed: false,
                    result: Ok(()),
                };
            }
            Err(error) => return failed(io_error(error)),
        };
        // Deleting only part of it would not free the path for the original.
        let compared = manifest::compare(
            fs(),
            parent.clone(),
            &placed.root(),
            unlinks.clone(),
            cancelled.clone(),
        );
        if compared.result.is_err() || !compared.kept.is_empty() {
            return compared;
        }
        manifest::remove_placed(fs(), parent, &placed.root(), unlinks, cancelled)
    })
    .await;
    removed.unwrap_or_else(|_| failed(translated_io_error("The deletion stopped unexpectedly")))
}

pub(super) struct Copied {
    pub ownership: Option<Ownership>,
    pub name: OsString,
    pub reported: Reported,
    pub manifest: Option<Manifest>,
    /// Barriers of the mounts nested in a durably merged folder.
    pub nested: Vec<Barrier>,
}

#[derive(Clone, Default)]
pub(super) struct Options {
    pub names: NamePolicy,
    pub workers: Option<usize>,
    pub manifest: bool,
    /// What only the copy will hold is deleted once its [`Barrier`] is reached,
    /// so files that barrier cannot reach are synced as written.
    pub durable: bool,
}

pub(super) async fn copy_local(
    source: PathBuf,
    destination: Destination,
    options: Options,
    progress: Option<Rc<TransferProgressTracker>>,
    cancellable: &gio::Cancellable,
) -> Result<Copied, glib::Error> {
    let (Destination::New(target, _) | Destination::Merge(target, _)) = &destination;
    let target = target.clone();
    let merging = matches!(destination, Destination::Merge(..));
    let workers = options.workers;
    let followed = following(
        Item::Path(source),
        options,
        progress.clone(),
        cancellable,
        move |copy, source| match destination {
            Destination::New(target, taken) => {
                let (parent, name) = split(&target).map_err(Failure::Error)?;
                let new = NewItem::Published(taken);
                let (name, ownership) = copy_new(copy, source, (&parent, &name), new, workers)?;
                let published = Some(Published::proven(parent, name.clone(), ownership));
                Ok((name, Vec::new(), published))
            }
            Destination::Merge(folder, placing) => {
                let name = folder.file_name().unwrap_or_default().to_os_string();
                merge(copy, source, &folder, workers, placing)
                    .map(|(nested, published)| (name, nested, published))
            }
        },
    )
    .await?;
    match followed.value {
        Ok((name, nested, published)) => {
            report(
                progress.as_deref(),
                &target.with_file_name(&name),
                followed.left,
            );
            let ownership = published.as_ref().map(|copy| copy.ownership().clone());
            let mut manifest = followed.manifest;
            if let (Some(manifest), Some(published)) = (&mut manifest, published) {
                manifest.copied_to(published);
            }
            Ok(Copied {
                ownership,
                name,
                reported: followed.reported,
                manifest,
                nested,
            })
        }
        // What a stopped merge placed stays.
        Err(error) => {
            if merging {
                report(progress.as_deref(), &target, followed.left);
            }
            Err(error)
        }
    }
}

pub(super) async fn copy_to_stage(
    source: PathBuf,
    target: PathBuf,
    options: Options,
    progress: Option<Rc<TransferProgressTracker>>,
    cancellable: &gio::Cancellable,
) -> Result<(StagedSibling, Option<Manifest>, TransferReport), glib::Error> {
    let root = target.clone();
    let workers = options.workers;
    let followed = following(
        Item::Path(source),
        options,
        progress.clone(),
        cancellable,
        move |copy, source| {
            let staged = copy_stage(copy, source, &target, workers)?;
            match copy.check_cancelled() {
                Ok(()) => Ok(staged),
                Err(failure) => Err(copy.discarding(staged, failure)),
            }
        },
    )
    .await?;
    let staged = followed.value?;
    Ok((staged, followed.manifest, left_report(&root, followed.left)))
}

/// For a mount that can place neither the item nor a stage. A failed copy
/// removes only what it made.
pub(super) async fn copy_in_place(
    (source, name, expected): (Arc<OwnedFd>, OsString, LocalFileIdentity),
    (parent, target): (Arc<OwnedFd>, PathBuf),
    options: Options,
    cancellable: &gio::Cancellable,
) -> Result<(Manifest, TransferReport), glib::Error> {
    let Some(target_name) = target.file_name().map(OsStr::to_os_string) else {
        return Err(io_error(crate::i18n::tr("Invalid local filesystem target")));
    };
    let options = Options {
        manifest: true,
        ..options
    };
    let workers = options.workers;
    let copied_into = OffMain::new(parent.clone());
    let followed = following(
        Item::Pinned(source, name),
        options,
        None,
        cancellable,
        move |copy, source| {
            let new = NewItem::InPlace(expected);
            copy_new(copy, source, (&parent, &target_name), new, workers)
        },
    )
    .await?;
    let (name, ownership) = followed.value?;
    let mut manifest = followed
        .manifest
        .ok_or_else(|| translated_io_error("The copy stopped unexpectedly"))?;
    manifest.copied_to(Published::proven(copied_into.into_inner(), name, ownership));
    Ok((manifest, left_report(&target, followed.left)))
}

pub(super) async fn snapshot(
    source: PathBuf,
    cancellable: &gio::Cancellable,
) -> Result<Manifest, glib::Error> {
    let recorded = run_cancellable(cancellable, move |cancelled| {
        let (parent, name) = split(&source)?;
        manifest::snapshot(fs(), parent, &name, &cancelled)
            .map_err(|failure| failure.into_error(&name))
    })
    .await;
    recorded.map_err(|_| translated_io_error("The copy stopped unexpectedly"))?
}

pub(super) async fn delete_unchanged_sources(
    manifest: Manifest,
    unlinks: Arc<Unlinks>,
    cancellable: &gio::Cancellable,
) -> (Vec<(PathBuf, KeptCause)>, Result<(), glib::Error>) {
    let deleted = run_cancellable(cancellable, move |cancelled| {
        manifest::delete(fs(), manifest, unlinks, cancelled)
    })
    .await;
    deleted.unwrap_or_else(|_| {
        (
            Vec::new(),
            Err(translated_io_error("The deletion stopped unexpectedly")),
        )
    })
}

pub(super) async fn sync_files(
    paths: Vec<PathBuf>,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let synced = run_cancellable(cancellable, move |cancelled| {
        for path in paths {
            let (parent, name) = split(&path)?;
            sync_tree(fs(), Entry::new(&*parent, &name), &cancelled)
                .map_err(|failure| failure.into_error(&name))?;
        }
        Ok(())
    })
    .await;
    synced.map_err(|_| translated_io_error("The copy stopped unexpectedly"))?
}

fn sync_tree(fs: &dyn FsOps, at: Entry<'_>, cancelled: &AtomicBool) -> Result<(), Failure> {
    if cancelled.load(Ordering::Acquire) {
        return Err(Failure::Cancelled);
    }
    match fs.stat(at, false)?.kind {
        FileType::Directory => {
            let dir = fs.open_dir(at)?;
            for (name, _) in fs.list(dir.as_fd())? {
                sync_tree(fs, Entry::new(&dir, &name), cancelled)
                    .map_err(|failure| failure.named(&name))?;
            }
        }
        FileType::RegularFile => {
            // An SMB server flushes only for a handle that may write.
            let file = match fs.open_write(at) {
                Err(Errno::ACCESS | Errno::PERM) => fs.open_source(at, true)?.map(|(file, _)| file),
                opened => Some(opened?),
            };
            if let Some(file) = file {
                fs.sync_data(file.as_fd())?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn file_ownership(stat: &EntryStat) -> Ownership {
    Ownership::of(stat)
}

pub(super) fn staged_ownership(staged: &StagedSibling) -> Ownership {
    staged
        .ownership
        .clone()
        .unwrap_or_else(|| Ownership::new(staged.identity))
}

pub(super) fn published(parent: Arc<OwnedFd>, name: OsString, ownership: Ownership) -> Published {
    Published::proven(parent, name, ownership)
}

pub(super) async fn placed_proven(
    path: PathBuf,
    ownership: Ownership,
    left: Arc<AtomicUsize>,
    cancellable: &gio::Cancellable,
) -> Option<PlacedSnapshot> {
    run_cancellable(cancellable, move |cancelled| {
        let (parent, name) = split(&path).ok()?;
        manifest::placed_proven(fs(), (parent.as_fd(), &name), &cancelled, &left, &ownership)
            .ok()
            .map(PlacedSnapshot::new)
    })
    .await
    .ok()
    .flatten()
}

pub(super) fn placed_at_proven(
    fs: &dyn FsOps,
    at: (BorrowedFd<'_>, &OsStr),
    ownership: &Ownership,
    cancelled: &AtomicBool,
) -> Option<PlacedSnapshot> {
    manifest::placed_proven(fs, at, cancelled, &manifest::placed_entries(), ownership)
        .ok()
        .map(PlacedSnapshot::new)
}

/// Must be opened before a copy writes beneath `path`.
pub(super) async fn barrier(path: PathBuf) -> Result<Barrier, glib::Error> {
    run_blocking(move || Barrier::open(fs(), &path)).await
}

pub(super) async fn reach(barrier: Barrier, removable: bool) -> Result<(), String> {
    match gio::spawn_blocking(move || barrier.reach(fs(), removable)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(os_error_text(error)),
        Err(error) => Err(blocking_join_message(error)),
    }
}

struct Followed<T> {
    value: Result<T, glib::Error>,
    reported: Reported,
    left: Left,
    manifest: Option<Manifest>,
}

#[derive(Default)]
struct Left {
    renamed: Vec<(PathBuf, PathBuf, RenameCause)>,
    skipped: Vec<(PathBuf, SpecialKind)>,
    item: Option<(OsString, RenameCause)>,
}

fn rebase(path: &mut PathBuf, from: &Path, to: &Path) {
    if let Ok(rest) = path.strip_prefix(from) {
        *path = if rest.as_os_str().is_empty() {
            to.to_path_buf()
        } else {
            to.join(rest)
        };
    }
}

fn report(progress: Option<&TransferProgressTracker>, root: &Path, left: Left) {
    if let Some(progress) = progress {
        progress.report(left_report(root, left));
    }
}

fn left_report(root: &Path, left: Left) -> TransferReport {
    let location = |path: PathBuf| Location::local(root.join(path));
    let item = left.item.map(|(requested, cause)| RenamedItem {
        requested: Location::local(root.with_file_name(requested)),
        saved_as: Location::local(root.to_path_buf()),
        cause,
    });
    TransferReport {
        renamed: item
            .into_iter()
            .chain(
                left.renamed
                    .into_iter()
                    .map(|(requested, saved_as, cause)| RenamedItem {
                        requested: location(requested),
                        saved_as: location(saved_as),
                        cause,
                    }),
            )
            .collect(),
        skipped: left
            .skipped
            .into_iter()
            .map(|(path, kind)| SkippedItem {
                location: location(path),
                kind,
            })
            .collect(),
        ..TransferReport::default()
    }
}

enum Item {
    Path(PathBuf),
    Pinned(Arc<OwnedFd>, OsString),
}

async fn following<T: Send + 'static>(
    item: Item,
    options: Options,
    progress: Option<Rc<TransferProgressTracker>>,
    cancellable: &gio::Cancellable,
    work: impl FnOnce(&Arc<Copy>, &Source) -> Result<T, Failure> + Send + 'static,
) -> Result<Followed<T>, glib::Error> {
    let shared = Arc::new(SharedProgress::default());
    let follower = progress.map(|progress| {
        let name = match &item {
            Item::Path(path) => path.file_name(),
            Item::Pinned(_, name) => Some(name.as_os_str()),
        };
        if let Some(name) = name {
            progress.set_current_file(name.to_string_lossy().into_owned());
        }
        progress.follow(&shared)
    });
    let counters = shared.clone();
    let copied = run_cancellable(cancellable, move |cancelled| {
        let (parent, name) = match item {
            Item::Path(path) => match split(&path) {
                Ok(split) => split,
                Err(error) => return (Err(error), Left::default(), None),
            },
            Item::Pinned(parent, name) => (parent, name),
        };
        let source = Source {
            parent,
            name,
            recorded_in: options.manifest.then(manifest::Folder::root),
        };
        let copy = Copy::new(
            fs(),
            source.name.clone(),
            cancelled,
            counters,
            true,
            options,
            Deadline::default(),
        );
        let value = work(&copy, &source).map_err(|failure| failure.into_error(&source.name));
        let manifest = source.recorded_in.map(|root| {
            let mut manifest = manifest::of(source.parent, root);
            for copy in std::mem::take(&mut *lock(&copy.copies)) {
                manifest.copied_to(copy);
            }
            manifest
        });
        (value, std::mem::take(&mut *lock(&copy.left)), manifest)
    })
    .await;
    let reported = follower.map_or_else(|| shared.reported(), |follower| follower.stop());
    let (value, left, manifest) =
        copied.map_err(|_| translated_io_error("The copy stopped unexpectedly"))?;
    Ok(Followed {
        value,
        reported,
        left,
        manifest,
    })
}

pub(super) async fn copy_from_stage(
    staged: &StagedSibling,
    name: OsString,
    durable: bool,
    cancellable: &gio::Cancellable,
) -> Result<Ownership, glib::Error> {
    let source = Source {
        parent: staged.parent.clone(),
        name: staged.name.clone(),
        recorded_in: None,
    };
    let identity = staged.identity;
    let copied = run_cancellable(cancellable, move |cancelled| {
        let copy = Copy::new(
            fs(),
            name.clone(),
            cancelled,
            Arc::default(),
            false,
            Options {
                durable,
                ..Options::default()
            },
            Deadline::default(),
        );
        let parent = source.parent.clone();
        from_stage(&copy, &source, identity, &parent, &name)
            .map_err(|failure| failure.into_error(&name))
    })
    .await;
    copied.map_err(|_| translated_io_error("The copy stopped unexpectedly"))?
}

pub(super) fn unlock_stage(staged: &StagedSibling) {
    if staged.restrictive.is_empty() {
        return;
    }
    let fs = fs();
    let Ok(root) = fs.open_dir(Entry::new(&*staged.parent, &staged.name)) else {
        return;
    };
    if fs
        .stat_fd(root.as_fd())
        .is_ok_and(|stat| stat.identity == staged.identity)
    {
        let mount = Mount::new(fs, root.as_fd());
        restrictive::loosen(fs, root.as_fd(), &staged.restrictive, &mount);
    }
}

struct Source {
    parent: Arc<OwnedFd>,
    name: OsString,
    recorded_in: Option<Arc<manifest::Folder>>,
}

impl Source {
    fn entry(&self) -> Entry<'_> {
        Entry::new(&*self.parent, &self.name)
    }
}

fn split(path: &Path) -> Result<(Arc<OwnedFd>, OsString), glib::Error> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io_error(crate::i18n::tr("Invalid local filesystem target")));
    };
    let parent = open_local_parent_directory(parent).map_err(io_error)?;
    Ok((Arc::new(parent), name.to_os_string()))
}

fn first_free<T>(
    copy: &Copy,
    dir: BorrowedFd<'_>,
    (name, taken, is_directory): (&OsStr, Taken, bool),
    mut attempt: impl FnMut(&OsStr) -> Result<T, Failure>,
) -> Result<(OsString, T), Failure> {
    for candidate in taken.names(name, is_directory, || copy.name_limit(dir)) {
        match attempt(&candidate) {
            Err(Failure::Place(PlaceError::Exists)) if taken == Taken::Number => {}
            Err(refused) if refused.refuses_name() => {
                let names = (name, candidate.as_os_str());
                return copy.retry_item(dir, (names, is_directory, taken), refused, attempt);
            }
            result => return result.map(|value| (candidate, value)),
        }
    }
    Err(Failure::Place(PlaceError::Exists))
}

/// A mount that learns keeps the rules that made a name. Fails as at first where none is
/// made.
fn retry_refused<T>(
    copy: &Copy,
    rules: &FolderRules,
    (name, is_directory): (&OsStr, bool),
    refused: Failure,
    mut make: impl FnMut(&OsStr) -> Result<(OsString, T), Failure>,
) -> Result<(OsString, T, RenameCause), Failure> {
    for (strictness, stricter, cause) in stricter_names(name, &rules.fixed, is_directory) {
        match make(&stricter) {
            Ok((made, value)) => {
                if rules.site.rules.learns {
                    copy.names.profiles.learn(&rules.site, strictness);
                }
                return Ok((made, value, cause));
            }
            Err(failure) if failure.refuses_name() => {}
            Err(failure) => return Err(failure),
        }
    }
    Err(refused)
}

#[derive(Clone, Copy)]
enum NewItem {
    Published(Taken),
    InPlace(LocalFileIdentity),
}

fn copy_new(
    copy: &Arc<Copy>,
    source: &Source,
    (parent, name): (&Arc<OwnedFd>, &OsStr),
    new: NewItem,
    workers: Option<usize>,
) -> Result<(OsString, Ownership), Failure> {
    let stat = copy.source_stat(source.entry())?;
    let taken = match new {
        NewItem::Published(taken) => taken,
        NewItem::InPlace(expected) if expected == stat.identity => Taken::Fail,
        NewItem::InPlace(_) => return Err(Failure::Changed),
    };
    let in_place = matches!(new, NewItem::InPlace(_));
    let recorded_in = source.recorded_in.as_deref();
    match stat.kind {
        FileType::RegularFile if in_place => FileCopy::new(copy, parent.as_fd())
            .in_place(source.entry(), &stat, name, recorded_in)
            .map(|identity| (name.to_os_string(), copy.ownership(identity))),
        FileType::RegularFile => FileCopy::new(copy, parent.as_fd())
            .copy_from(
                source.entry(),
                &stat,
                FileTarget::Final(name, taken),
                recorded_in,
            )
            .map(|(name, identity)| (name, copy.ownership(identity))),
        FileType::Symlink | FileType::Fifo => {
            copy_special(copy, source, (parent, name), &stat, taken)
        }
        FileType::Directory => {
            let workers = workers.unwrap_or_else(|| worker_count(source, parent));
            if in_place || placement::can_place(copy.fs, parent.as_fd(), true) == Some(false) {
                return first_free(copy, parent.as_fd(), (name, taken, true), |name| {
                    direct_folder(copy, source, stat.identity, parent, name, workers)
                });
            }
            let staged = stage_folder(copy, source, stat.identity, parent, workers)?;
            publish_folder(copy, staged, name, taken, workers)
        }
        _ => Err(Failure::NotCopyable),
    }
}

fn copy_special(
    copy: &Arc<Copy>,
    source: &Source,
    (parent, name): (&Arc<OwnedFd>, &OsStr),
    stat: &EntryStat,
    taken: Taken,
) -> Result<(OsString, Ownership), Failure> {
    copy.check_cancelled()?;
    let mount = copy.mount(parent.as_fd());
    let link = stat.kind == FileType::Symlink;
    let site = copy.site(&mount, parent.as_fd());
    if link && !copy.names.profiles.holds_links(&site) {
        return Err(Failure::LinkRefused(Errno::OPNOTSUPP));
    }
    let target = link
        .then(|| tree::read_link(copy.fs, source.entry()))
        .transpose()?;
    if let Some(target) = &target
        && !copy.names.profiles.holds_link_to(&site, target)
    {
        return Err(Failure::LinkRefused(Errno::ILSEQ));
    }
    let content = match &target {
        Some(target) => StageContent::Symlink(target.clone()),
        None => StageContent::Fifo,
    };
    // These creation syscalls return no descriptor; retain ownership in a private stage.
    let staged = StagedSibling::create(parent.clone(), content).map_err(|error| {
        match error {
            PlaceError::Io(error)
                if stat.kind == FileType::Fifo && tree::refuses_node(FileType::Fifo, error) =>
            {
                Failure::NotCopyable
            }
            // The stage's name is valid everywhere, unlike the target.
            PlaceError::Io(error)
                if link
                    && (tree::refuses_node(FileType::Symlink, error)
                        || matches!(error, Errno::INVAL | Errno::ILSEQ)) =>
            {
                let refused = Failure::LinkRefused(error);
                copy.refused_link(&site, &refused);
                refused
            }
            error => error.into(),
        }
    })?;
    let prepared = (|| {
        let at = Entry::new(&**parent, &staged.name);
        if stat.kind == FileType::Fifo {
            let parent_stat = copy.fs.stat_fd(parent.as_fd())?;
            tree::finish_pipe(copy, stat, (at, staged.identity), &parent_stat, &mount)?;
        } else {
            if mount.family != Family::Fuse {
                best_effort(copy.fs.set_link_times(at, stat.atime, stat.mtime))?;
            }
            if copy.recording {
                copy.record_state(staged.identity, &copy.fs.stat(at, false)?);
            }
        }
        copy.check_cancelled()
    })();
    if let Err(failure) = prepared {
        return Err(copy.discarding(staged, failure));
    }
    let published =
        first_free(
            copy,
            parent.as_fd(),
            (name, taken, false),
            |name| match stage::publish(
                copy.fs,
                parent.as_fd(),
                &staged.name,
                staged.identity,
                name,
            ) {
                Ok(placement::Placed::RecreatedSymlink) => Ok((
                    tree::made_again(copy.fs, Entry::new(&**parent, name), target.as_deref()),
                    false,
                )),
                Ok(_) => Ok((copy.ownership(staged.identity), false)),
                Err(PlaceError::Unsupported) => {
                    let from = Entry::new(&**parent, &staged.name);
                    let to = Entry::new(&**parent, name);
                    let uncounted = copy.uncounted();
                    if stat.kind == FileType::Symlink {
                        tree::copy_link(&uncounted, from, to, &mount, None)?;
                    } else {
                        let parent_stat = copy.fs.stat_fd(parent.as_fd())?;
                        if tree::copy_fifo(&uncounted, from, to, &parent_stat, &mount, None)?
                            .1
                            .is_none()
                        {
                            return Err(Failure::NotCopyable);
                        }
                    }
                    Ok((Ownership::unproven(), true))
                }
                Err(error) => Err(error.into()),
            },
        );
    match published {
        Ok((name, (ownership, copied_out))) => {
            if copied_out {
                copy.discard(staged)
                    .map_err(|error| Failure::Error(stage_left(&error)))?;
            }
            if let Some(folder) = &source.recorded_in {
                folder.entry(&source.name, stat);
            }
            copy.add_file(if stat.kind == FileType::Symlink {
                stat.size
            } else {
                0
            });
            Ok((name, ownership))
        }
        Err(Failure::Place(PlaceError::Unconfirmed)) => {
            Err(Failure::Place(PlaceError::Unconfirmed))
        }
        Err(failure) => {
            let failure = match link.then(|| copy.refused_link(&site, &failure)).flatten() {
                Some(error) => Failure::LinkRefused(error),
                None => failure,
            };
            Err(copy.discarding(staged, failure))
        }
    }
}

fn copy_stage(
    copy: &Arc<Copy>,
    source: &Source,
    target: &Path,
    workers: Option<usize>,
) -> Result<StagedSibling, Failure> {
    let (parent, _) = split(target).map_err(Failure::Error)?;
    let stat = copy.source_stat(source.entry())?;
    let recorded_in = source.recorded_in.as_deref();
    match stat.kind {
        FileType::RegularFile => {
            let (name, identity) = FileCopy::new(copy, parent.as_fd()).copy_from(
                source.entry(),
                &stat,
                FileTarget::Hidden,
                recorded_in,
            )?;
            Ok(StagedSibling {
                ownership: Some(copy.ownership(identity)),
                parent,
                name,
                identity,
                restrictive: Vec::new(),
            })
        }
        FileType::Symlink => {
            copy.check_cancelled()?;
            let site = copy.site(&copy.mount(parent.as_fd()), parent.as_fd());
            if !copy.names.profiles.holds_links(&site) {
                return Err(Failure::LinkRefused(Errno::OPNOTSUPP));
            }
            let link = tree::read_link(copy.fs, source.entry())?;
            if !copy.names.profiles.holds_link_to(&site, &link) {
                return Err(Failure::LinkRefused(Errno::ILSEQ));
            }
            let staged = match StagedSibling::create(parent, StageContent::Symlink(link)) {
                // The stage's name is valid everywhere, unlike the target.
                Err(PlaceError::Io(error))
                    if tree::refuses_node(FileType::Symlink, error)
                        || matches!(error, Errno::INVAL | Errno::ILSEQ) =>
                {
                    let refused = Failure::LinkRefused(error);
                    copy.refused_link(&site, &refused);
                    return Err(refused);
                }
                staged => staged?,
            };
            // sshfs sets a link's times on what it points to.
            if Mount::new(copy.fs, staged.parent.as_fd()).family != Family::Fuse {
                let link = Entry::new(&*staged.parent, &staged.name);
                if let Err(error) =
                    best_effort(copy.fs.set_link_times(link, stat.atime, stat.mtime))
                {
                    return Err(copy.discarding(staged, error.into()));
                }
            }
            copy.add_file(stat.size);
            if let Some(folder) = recorded_in {
                folder.entry(&source.name, &stat);
            }
            Ok(staged)
        }
        FileType::Directory => {
            let workers = workers.unwrap_or_else(|| worker_count(source, &parent));
            stage_folder(copy, source, stat.identity, &parent, workers)
        }
        _ => Err(Failure::NotCopyable),
    }
}

fn worker_count(source: &Source, target: &OwnedFd) -> usize {
    let available = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let rotational = [source.parent.as_fd(), target.as_fd()]
        .into_iter()
        .any(|fd| fs().rotational(fd) == Some(true));
    bounded_local_worker_count(available, rotational)
}

fn stage_folder(
    copy: &Arc<Copy>,
    source: &Source,
    expected: LocalFileIdentity,
    parent: &Arc<OwnedFd>,
    workers: usize,
) -> Result<StagedSibling, Failure> {
    copy.check_cancelled()?;
    let mut staged = StagedSibling::create(parent.clone(), StageContent::Directory)?;
    let root = Root {
        parent: parent.clone(),
        name: staged.name.clone(),
        identity: staged.identity,
    };
    let created = Arc::new(Created::default());
    match fill(copy, source, expected, &root, workers, Some(&created)) {
        Ok(restrictive) => {
            staged.ownership =
                Some(copy.created_ownership(staged.identity, lock(&created.0).clone()));
            staged.restrictive = restrictive;
            Ok(staged)
        }
        Err(failure) => Err(copy.discarding(staged, failure)),
    }
}

fn publish_folder(
    copy: &Arc<Copy>,
    staged: StagedSibling,
    name: &OsStr,
    taken: Taken,
    workers: usize,
) -> Result<(OsString, Ownership), Failure> {
    let ownership = staged
        .ownership
        .clone()
        .unwrap_or_else(|| Ownership::new(staged.identity));
    if let Err(failure) = copy.check_cancelled() {
        return Err(copy.discarding(staged, failure));
    }
    let source = Source {
        parent: staged.parent.clone(),
        name: staged.name.clone(),
        recorded_in: None,
    };
    let mut copied_from = None;
    let published = first_free(copy, staged.parent.as_fd(), (name, taken, true), |name| {
        if copied_from.is_none() {
            match stage::publish(
                copy.fs,
                staged.parent.as_fd(),
                &staged.name,
                staged.identity,
                name,
            ) {
                Err(PlaceError::Unsupported) => {}
                published => {
                    return published.map(|_| ownership.clone()).map_err(Failure::Place);
                }
            }
        }
        let from = copied_from.get_or_insert_with(|| copy.uncounted());
        direct_folder(
            from,
            &source,
            staged.identity,
            &staged.parent,
            name,
            workers,
        )
    });
    match published {
        Ok(published) if copied_from.is_some() => match copy.discard(staged) {
            Ok(()) => Ok(published),
            Err(error) => Err(Failure::Error(stage_left(&error))),
        },
        Ok(published) => Ok(published),
        // Either name may hold the copy, so neither is removed.
        Err(Failure::Place(PlaceError::Unconfirmed)) if copied_from.is_none() => {
            Err(Failure::Place(PlaceError::Unconfirmed))
        }
        Err(failure) => Err(copy.discarding(staged, failure)),
    }
}

fn stage_left(error: &glib::Error) -> glib::Error {
    io_error(rust_i18n::t!(
        "The item was copied, but its staging copy could not be removed: %{error}",
        error = crate::services::gio_error_detail(error)
    ))
}

struct Root {
    parent: Arc<OwnedFd>,
    name: OsString,
    identity: LocalFileIdentity,
}

fn fill(
    copy: &Arc<Copy>,
    source: &Source,
    expected: LocalFileIdentity,
    root: &Root,
    workers: usize,
    created: Option<&Arc<Created>>,
) -> Result<Vec<Restrictive>, Failure> {
    let fs = copy.fs;
    let parent = fs.stat_fd(root.parent.as_fd())?;
    let mount = copy.mount(root.parent.as_fd());
    let folder = tree::Folder {
        source: (source.parent.clone(), source.name.clone(), Some(expected)),
        target: (root.parent.clone(), root.name.clone(), root.identity),
        parent,
        path: PathBuf::new(),
        naming: tree::Naming::Decide,
        site: copy.site(&mount, root.parent.as_fd()),
        mount,
        scope: 0,
        recorded_in: source.recorded_in.clone(),
        completion: None,
    };
    let (
        walked,
        tree::Walked {
            mut restrictive,
            links,
            ..
        },
    ) = tree::walk(copy, workers, created.cloned(), |tree| {
        tree.probe_links_in(&mount, root.parent.clone());
        tree::TreeJob::Folder(tree, folder)
    });
    walked.map_err(Failure::Error)?;
    if restrictive.is_empty() && links.is_empty() {
        return Ok(restrictive);
    }
    let opened = fs.open_dir(Entry::new(&*root.parent, &root.name))?;
    if !mount.may_be(root.identity, &fs.stat_fd(opened.as_fd())?) {
        return Err(Failure::Place(PlaceError::Changed));
    }
    links.make(
        0,
        copy,
        opened.as_fd(),
        Path::new(""),
        &mount,
        created.map(|created| &**created),
    )?;
    if let Err(error) = restrictive::tighten(fs, opened.as_fd(), &mut restrictive, mount.identity) {
        restrictive::loosen(fs, opened.as_fd(), &restrictive, &mount);
        return Err(error.into());
    }
    Ok(restrictive)
}

#[derive(Default)]
struct Created(Mutex<Vec<(PathBuf, LocalFileIdentity)>>);

impl Created {
    fn add(&self, path: PathBuf, identity: LocalFileIdentity) {
        lock(&self.0).push((path, identity));
    }

    fn remove(&self, copy: &Copy, root: &Root, mount: &Mount) -> Result<(), glib::Error> {
        let fs = copy.fs;
        let root_entry = Entry::new(&*root.parent, &root.name);
        let dir = match fs.open_dir(root_entry) {
            Ok(dir) => dir,
            Err(Errno::NOENT) => return Ok(()),
            Err(error) => return Err(io_error(os_error_detail(error))),
        };
        let opened = fs
            .stat_fd(dir.as_fd())
            .map_err(|error| io_error(os_error_detail(error)))?;
        if !mount.may_be(root.identity, &opened) {
            return Err(placement_error(PlaceError::Changed));
        }
        let mut made = std::mem::take(&mut *lock(&self.0));
        made.sort_by(|(left, _), (right, _)| {
            let depth = |path: &Path| path.components().count();
            depth(right)
                .cmp(&depth(left))
                .then_with(|| left.parent().cmp(&right.parent()))
        });
        let mut folder: Option<(&Path, OwnedFd)> = None;
        for (path, identity) in &made {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                continue;
            };
            if folder.as_ref().is_none_or(|(opened, _)| *opened != parent) {
                folder = fs
                    .open_dir(Entry {
                        dir: dir.as_fd(),
                        name: restrictive::beneath(parent),
                    })
                    .ok()
                    .map(|fd| (parent, fd));
            }
            let Some((_, fd)) = &folder else {
                continue;
            };
            let entry = Entry::new(fd, name);
            if identity.kind != FileType::Directory {
                let attributes = copy.written_attributes(*identity);
                placement::remove_if_unchanged(fs, entry, *identity, attributes, AtFlags::empty());
                continue;
            }
            let unchanged = || {
                fs.stat(entry, true)
                    .is_ok_and(|current| current.identity == *identity)
            };
            if unchanged() {
                let _ = copy.remove_folder(entry, &unchanged);
            }
        }
        drop((folder, dir));
        let made = |current: &EntryStat| mount.may_be(root.identity, current);
        let unchanged = || {
            fs.stat(root_entry, true)
                .is_ok_and(|current| made(&current))
        };
        remove_own(fs, root_entry, made, |at| {
            copy.remove_folder(at, &unchanged)
        })
    }
}

fn direct_folder(
    copy: &Arc<Copy>,
    source: &Source,
    expected: LocalFileIdentity,
    parent: &Arc<OwnedFd>,
    name: &OsStr,
    workers: usize,
) -> Result<Ownership, Failure> {
    let fs = copy.fs;
    copy.check_cancelled()?;
    let target = Entry::new(&**parent, name);
    if let Err(error) = fs.mkdir(target, Mode::from_raw_mode(0o700)) {
        return Err(if tree::taken(fs, target, error) {
            Failure::Place(PlaceError::Exists)
        } else {
            creation_failure(fs, target, error)
        });
    }
    let identity = fs
        .open_dir(target)
        .and_then(|fd| fs.stat_fd(fd.as_fd()))?
        .identity;
    let root = Root {
        parent: parent.clone(),
        name: name.to_os_string(),
        identity,
    };
    let created = Arc::new(Created::default());
    fill(copy, source, expected, &root, workers, Some(&created))
        .map(|_| copy.created_ownership(identity, lock(&created.0).clone()))
        .map_err(|failure| {
            let mount = Mount::new(fs, parent.as_fd());
            let removed = created.remove(copy, &root, &mount);
            copy.after_cleanup(failure, removed)
        })
}

fn from_stage(
    copy: &Arc<Copy>,
    source: &Source,
    identity: LocalFileIdentity,
    parent: &Arc<OwnedFd>,
    name: &OsStr,
) -> Result<Ownership, Failure> {
    match identity.kind {
        FileType::Directory => {
            let workers = worker_count(source, parent);
            direct_folder(copy, source, identity, parent, name, workers)
        }
        FileType::RegularFile => {
            let parent_stat = copy.fs.stat_fd(parent.as_fd())?;
            FileCopy::new(copy, parent.as_fd())
                .create_from(source.entry(), identity, None, name, &parent_stat)
                .map(|identity| copy.ownership(identity))
        }
        // The ladder recreates symlinks itself.
        _ => Err(Failure::Place(PlaceError::Unsupported)),
    }
}

fn merge(
    copy: &Arc<Copy>,
    source: &Source,
    folder: &Path,
    workers: Option<usize>,
    placing: Option<Arc<Placing>>,
) -> Result<(Vec<Barrier>, Option<Published>), Failure> {
    let (parent, name) = split(folder).map_err(Failure::Error)?;
    let workers = workers.unwrap_or_else(|| worker_count(source, &parent));
    let merged = tree::Merged {
        source: (source.parent.clone(), source.name.clone()),
        target: (parent.clone(), name.clone(), None),
        path: PathBuf::new(),
        recorded_in: source.recorded_in.clone(),
        placing,
        parent: None,
    };
    let (
        walked,
        tree::Walked {
            stages,
            placed,
            mut nested,
            copies,
            ..
        },
    ) = tree::walk(copy, workers, None, |tree| {
        tree::TreeJob::Merge(tree, merged)
    });
    let Err(error) = walked else {
        for barrier in &mut nested {
            barrier.path = folder.join(&barrier.path);
        }
        lock(&copy.copies).extend(copies);
        return Ok((nested, None));
    };
    copy.keep_left_beneath(&placed);
    let mut cleanup = Ok(());
    for staged in stages {
        cleanup = cleanup.and(copy.discard(staged));
    }
    Err(copy.after_cleanup(Failure::Error(error), cleanup))
}

enum Failure {
    Cancelled,
    Os(Errno),
    NameRefused(Errno),
    LinkRefused(Errno),
    Place(PlaceError),
    Changed,
    NotCopyable,
    Error(glib::Error),
}

impl From<Errno> for Failure {
    fn from(error: Errno) -> Self {
        Self::Os(error)
    }
}

impl From<PlaceError> for Failure {
    fn from(error: PlaceError) -> Self {
        match error {
            PlaceError::Io(errno) if !error.refuses_name() => Self::Os(errno),
            error => Self::Place(error),
        }
    }
}

impl Failure {
    fn named(self, name: &OsStr) -> Self {
        Self::Error(self.into_error(name))
    }

    fn refuses_name(&self) -> bool {
        match self {
            Self::NameRefused(_) => true,
            Self::Place(error) => error.refuses_name(),
            _ => false,
        }
    }

    fn into_error(self, name: &OsStr) -> glib::Error {
        let name = name.to_string_lossy();
        match self {
            Self::Cancelled => cancelled_local_operation(),
            Self::Os(error) => {
                let message = rust_i18n::t!(
                    "Could not copy %{name}: %{error}",
                    name = name,
                    error = os_error_detail(error)
                );
                match error {
                    Errno::NOSPC | Errno::DQUOT => {
                        glib::Error::new(gio::IOErrorEnum::NoSpace, &message)
                    }
                    _ => io_error(message),
                }
            }
            Self::NameRefused(error) => refused_name(&name, error),
            Self::LinkRefused(error) => glib::Error::new(
                Unheld::Symlink,
                &rust_i18n::t!(
                    "Could not copy %{name}: %{error}",
                    name = name,
                    error = os_error_detail(error)
                ),
            ),
            Self::Place(error @ PlaceError::Io(errno)) if error.refuses_name() => {
                refused_name(&name, errno)
            }
            Self::Place(PlaceError::IntoItself) => placement_error(PlaceError::IntoItself),
            Self::Place(PlaceError::Changed) => io_error(rust_i18n::t!(
                "%{name} changed while the operation was in progress",
                name = name
            )),
            Self::Place(error) => glib::Error::new(
                placement_error_kind(error),
                &rust_i18n::t!(
                    "Could not copy %{name}: %{error}",
                    name = name,
                    error = crate::services::error_detail(placement_reason(error))
                ),
            ),
            Self::Changed => io_error(rust_i18n::t!(
                "%{name} changed while it was being copied",
                name = name
            )),
            Self::NotCopyable => io_error(rust_i18n::t!(
                "Cannot copy %{name}: it is not a regular file, directory, or symbolic link",
                name = name
            )),
            Self::Error(error) => error,
        }
    }
}

fn refused_name(name: &str, error: Errno) -> glib::Error {
    let kind = if error == Errno::NAMETOOLONG {
        gio::IOErrorEnum::FilenameTooLong
    } else {
        gio::IOErrorEnum::InvalidFilename
    };
    let message = rust_i18n::t!(
        "Could not copy %{name}: %{error}",
        name = name,
        error = crate::services::error_detail(refusal_reason(error))
    );
    glib::Error::new(kind, &message)
}

pub(super) fn refusal_reason(error: Errno) -> String {
    if error == Errno::NAMETOOLONG {
        os_error_text(error)
    } else {
        crate::i18n::tr(INVALID_NAME_REASON)
    }
}

/// SMB and exfat-fuse refuse a name with `ENOENT` in a folder that still exists.
fn creation_failure(fs: &dyn FsOps, at: Entry<'_>, error: Errno) -> Failure {
    if refuses_name(fs, at.dir, error) {
        Failure::NameRefused(error)
    } else {
        Failure::Os(error)
    }
}

pub(super) fn refuses_name(fs: &dyn FsOps, dir: BorrowedFd<'_>, error: Errno) -> bool {
    match error {
        Errno::INVAL | Errno::ILSEQ | Errno::NAMETOOLONG | Errno::BADMSG => true,
        Errno::NOENT => fs.stat_fd(dir).is_ok_and(|folder| folder.nlink > 0),
        _ => false,
    }
}

fn is_fatal(error: Errno) -> bool {
    matches!(
        error,
        Errno::IO
            | Errno::STALE
            | Errno::NOTCONN
            | Errno::CONNABORTED
            | Errno::CONNRESET
            | Errno::TIMEDOUT
            | Errno::HOSTDOWN
            | Errno::HOSTUNREACH
            | Errno::NETDOWN
            | Errno::NETUNREACH
    )
}

fn best_effort(result: Result<(), Errno>) -> Result<(), Errno> {
    match result {
        Err(error) if is_fatal(error) => Err(error),
        _ => Ok(()),
    }
}

fn is_unsupported(error: Errno) -> bool {
    matches!(error, Errno::OPNOTSUPP | Errno::NOTTY | ENOTSUPP)
}

pub(super) fn holds_no_links(error: Errno) -> bool {
    error == Errno::NOSYS || is_unsupported(error)
}

fn remove_own<'a>(
    fs: &dyn FsOps,
    entry: Entry<'a>,
    made: impl FnOnce(&EntryStat) -> bool,
    remove: impl FnOnce(Entry<'a>) -> Result<(), leftovers::Refused>,
) -> Result<(), glib::Error> {
    match fs.stat(entry, true) {
        Ok(current) if made(&current) => match remove(entry) {
            Ok(()) | Err(leftovers::Refused::Os(Errno::NOENT)) => Ok(()),
            Err(leftovers::Refused::Changed(_)) => Err(placement_error(PlaceError::Changed)),
            Err(refused) => Err(io_error(os_error_detail(refused.errno()))),
        },
        Ok(_) => Err(placement_error(PlaceError::Changed)),
        Err(Errno::NOENT) => Ok(()),
        Err(error) => Err(io_error(os_error_detail(error))),
    }
}

#[derive(Clone, Copy)]
struct Mount {
    key: MountKey,
    family: Family,
    identity: IdentityTraits,
    fuse_format: Option<FuseFormat>,
    coarse_times: Option<CoarseTimes>,
    deferred_close: Option<Duration>,
    attribute_cache: Option<Duration>,
    names: Option<MountNames>,
}

impl Mount {
    fn new(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> Self {
        let (key, capabilities) = known_mount(fs, dir);
        Self::with(key, capabilities)
    }

    fn with(key: MountKey, capabilities: MountCapabilities) -> Self {
        Self {
            key,
            family: capabilities.family.unwrap_or(Family::Other),
            identity: capabilities.identity,
            fuse_format: capabilities.fuse_format,
            coarse_times: capabilities.coarse_times,
            deferred_close: capabilities.deferred_close,
            attribute_cache: capabilities.attribute_cache,
            names: capabilities.names,
        }
    }

    /// The SMB client answers stats from attributes it cached, whose change time can predate
    /// one the server set since, such as once the deferred close of a file written there lands.
    fn caches_attributes(&self) -> bool {
        self.attribute_cache.is_some()
    }

    fn identity_now(&self, fs: &dyn FsOps, dir: BorrowedFd<'_>) -> IdentityTraits {
        mount_profile::identity_now(fs, dir, self.family, self.identity)
    }

    fn may_be(&self, recorded: LocalFileIdentity, current: &EntryStat) -> bool {
        compare(self.identity, recorded, None, current) != Proof::Different
    }

    fn capabilities(&self) -> MountCapabilities {
        mount_capabilities(self.key)
    }

    fn learn(&self, learn: impl FnOnce(&mut MountCapabilities)) {
        learn_mount_capability(self.key, learn);
    }
}

fn link_tmpfile(
    fs: &dyn FsOps,
    mount: &Mount,
    fd: BorrowedFd<'_>,
    to: Entry<'_>,
) -> Result<(), Errno> {
    let through_proc = mount.capabilities().tmpfile_link_through_proc;
    if through_proc != Some(true) {
        match fs.link_fd(fd, to, false) {
            Ok(()) => {
                if through_proc.is_none() {
                    mount
                        .learn(|capabilities| capabilities.tmpfile_link_through_proc = Some(false));
                }
                return Ok(());
            }
            // Once the plain form has worked, ENOENT is about the folder.
            Err(Errno::NOENT) if through_proc.is_none() => {}
            Err(error) => return Err(error),
        }
    }
    match fs.link_fd(fd, to, true) {
        Ok(()) => {
            if through_proc.is_none() {
                mount.learn(|capabilities| capabilities.tmpfile_link_through_proc = Some(true));
            }
            Ok(())
        }
        // Unless the folder itself was removed meanwhile, or the mount may
        // refuse the name, as SMB and exfat-fuse do with `ENOENT`.
        Err(Errno::NOENT)
            if through_proc.is_none()
                && !may_be_refused(to.name)
                && fs.stat_fd(to.dir).is_ok_and(|folder| folder.nlink > 0) =>
        {
            mount.learn(|capabilities| capabilities.tmpfile = Some(false));
            Err(Errno::NOENT)
        }
        Err(error) => Err(error),
    }
}

struct Copy {
    fs: &'static dyn FsOps,
    item: OsString,
    cancelled: Arc<AtomicBool>,
    progress: Arc<SharedProgress>,
    /// False for a copy from a stage, whose bytes were counted already and
    /// which is removed afterwards, so the copy may leave nothing out.
    counted: bool,
    names: NamePolicy,
    data: Mutex<Vec<Data>>,
    refused_xattrs: Mutex<HashSet<OsString>>,
    nodes_refused: AtomicBool,
    recording: bool,
    durable: bool,
    left: Mutex<Left>,
    copies: Mutex<Vec<Published>>,
    written: Mutex<HashMap<(u64, u64), (LocalFileIdentity, manifest::State)>>,
    leftovers: Deadline,
}

impl Copy {
    fn new(
        fs: &'static dyn FsOps,
        item: OsString,
        cancelled: Arc<AtomicBool>,
        progress: Arc<SharedProgress>,
        counted: bool,
        options: Options,
        leftovers: Deadline,
    ) -> Arc<Self> {
        Arc::new(Self {
            fs,
            item,
            cancelled,
            progress,
            counted,
            names: options.names,
            data: Mutex::default(),
            refused_xattrs: Mutex::default(),
            nodes_refused: AtomicBool::new(false),
            recording: options.manifest,
            durable: options.durable,
            left: Mutex::default(),
            copies: Mutex::default(),
            written: Mutex::default(),
            leftovers,
        })
    }

    fn ownership(&self, identity: LocalFileIdentity) -> Ownership {
        self.created_ownership(identity, std::iter::empty())
    }

    fn created_ownership(
        &self,
        identity: LocalFileIdentity,
        descendants: impl IntoIterator<Item = (PathBuf, LocalFileIdentity)>,
    ) -> Ownership {
        let descendants = descendants.into_iter().collect::<Vec<_>>();
        let written = lock(&self.written);
        let states = std::iter::once((PathBuf::new(), identity))
            .chain(descendants.iter().cloned())
            .filter_map(|(path, identity)| {
                let (known, state) = written.get(&(identity.device, identity.inode))?;
                (*known == identity).then_some((path, *state))
            })
            .collect::<Vec<_>>();
        Ownership::created(identity, descendants).with_states(states)
    }

    fn uncounted(&self) -> Arc<Self> {
        Self::new(
            self.fs,
            self.item.clone(),
            self.cancelled.clone(),
            self.progress.clone(),
            false,
            Options {
                names: self.names.clone(),
                durable: self.durable,
                ..Options::default()
            },
            self.leftovers.clone(),
        )
    }

    fn remove_folder(
        &self,
        at: Entry<'_>,
        unchanged: &dyn Fn() -> bool,
    ) -> Result<(), leftovers::Refused> {
        leftovers::remove_folder(self.fs, at, &self.leftovers, &|| false, unchanged)
    }

    /// Forced where the mount caches attributes: a move records the source from this stat.
    fn source_stat(&self, at: Entry<'_>) -> Result<EntryStat, Errno> {
        let forced = self.recording && Mount::new(self.fs, at.dir).caches_attributes();
        self.fs.stat(at, forced)
    }

    fn mount(&self, dir: BorrowedFd<'_>) -> Mount {
        let (key, capabilities) = self.names.profiles.mount(self.fs, dir);
        Mount::with(key, capabilities)
    }

    fn name_limit(&self, dir: BorrowedFd<'_>) -> NameLimit {
        self.folder_rules(dir).limit
    }

    fn folder_rules(&self, dir: BorrowedFd<'_>) -> FolderRules {
        self.names.in_folder(&self.site(&self.mount(dir), dir))
    }

    fn site(&self, mount: &Mount, dir: BorrowedFd<'_>) -> Site {
        self.names.profiles.site(mount.key, mount.names, dir)
    }

    /// Names given back, as undo does, stay refused. A name planned otherwise than for Keep Both
    /// is reported by the planner.
    fn retry_item<T>(
        &self,
        dir: BorrowedFd<'_>,
        ((name, refused_name), is_directory, taken): ((&OsStr, &OsStr), bool, Taken),
        refused: Failure,
        mut attempt: impl FnMut(&OsStr) -> Result<T, Failure>,
    ) -> Result<(OsString, T), Failure> {
        if self.names.mode != NameMode::Fix {
            return Err(refused);
        }
        let rules = self.folder_rules(dir);
        let target = (refused_name, is_directory);
        let (made, value, cause) = retry_refused(self, &rules, target, refused, |stricter| {
            let numbered = numbered_names(stricter, is_directory, rules.limit);
            for candidate in std::iter::once(stricter.to_os_string()).chain(numbered) {
                match attempt(&candidate) {
                    Err(Failure::Place(PlaceError::Exists)) => {}
                    made => return made.map(|value| (candidate, value)),
                }
            }
            Err(Failure::Place(PlaceError::Exists))
        })?;
        if taken == Taken::Fail || *name == *self.item {
            lock(&self.left).item = Some((name.to_os_string(), cause));
        }
        Ok((made, value))
    }

    /// A target the mount refuses fails as a name stricter rules could not help. A mount without
    /// links at all learns.
    fn refused_link(&self, site: &Site, failure: &Failure) -> Option<Errno> {
        let error = match failure {
            Failure::LinkRefused(error)
            | Failure::NameRefused(error @ (Errno::INVAL | Errno::ILSEQ))
            | Failure::Place(PlaceError::Io(error @ Errno::ILSEQ)) => *error,
            Failure::Place(PlaceError::InvalidName) => Errno::INVAL,
            _ => return None,
        };
        if holds_no_links(error) {
            self.names.profiles.refuse_links(site);
        }
        Some(error)
    }

    fn nested(&self) -> Arc<Self> {
        let options = Options {
            names: self.names.clone(),
            workers: None,
            manifest: self.recording,
            durable: self.durable,
        };
        Self::new(
            self.fs,
            self.item.clone(),
            self.cancelled.clone(),
            self.progress.clone(),
            self.counted,
            options,
            self.leftovers.clone(),
        )
    }

    fn adopt(&self, nested: &Self, path: &Path) {
        let Left {
            renamed, skipped, ..
        } = std::mem::take(&mut *lock(&nested.left));
        let mut left = lock(&self.left);
        left.renamed
            .extend(renamed.into_iter().map(|(requested, saved_as, cause)| {
                (path.join(requested), path.join(saved_as), cause)
            }));
        left.skipped
            .extend(skipped.into_iter().map(|(at, kind)| (path.join(at), kind)));
    }

    fn renamed(&self, requested: PathBuf, saved_as: PathBuf, cause: RenameCause) {
        lock(&self.left).renamed.push((requested, saved_as, cause));
    }

    fn relocate(&self, from: &Path, to: &Path) {
        let move_beneath = |path: &mut PathBuf| {
            if path != from {
                rebase(path, from, to);
            }
        };
        let mut left = lock(&self.left);
        for (requested, saved_as, _) in &mut left.renamed {
            move_beneath(requested);
            move_beneath(saved_as);
        }
        for (path, _) in &mut left.skipped {
            move_beneath(path);
        }
    }

    fn keep_left_beneath(&self, paths: &[PathBuf]) {
        let kept = |path: &Path| paths.iter().any(|placed| path.starts_with(placed));
        let mut left = lock(&self.left);
        left.renamed.retain(|(_, saved_as, _)| kept(saved_as));
        left.skipped.retain(|(path, _)| kept(path));
    }

    fn skip(
        &self,
        path: PathBuf,
        (name, recorded_in): (&OsStr, Option<&manifest::Folder>),
        kind: SpecialKind,
    ) -> Result<(), Failure> {
        if !self.counted {
            return Err(Failure::NotCopyable);
        }
        match recorded_in {
            Some(folder) => folder.keep(name, KeptCause::Skipped(kind)),
            None => lock(&self.left).skipped.push((path, kind)),
        }
        Ok(())
    }

    fn add_file(&self, bytes: u64) {
        if self.counted {
            self.progress.add_bytes(bytes);
            self.progress.add_file();
        }
    }

    fn sync_written(&self, fd: BorrowedFd<'_>, mount: &Mount) -> Result<(), Failure> {
        if self.durable && barrier::each_file(mount) {
            self.fs.sync_data(fd)?;
        }
        Ok(())
    }

    fn check_cancelled(&self) -> Result<(), Failure> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Failure::Cancelled);
        }
        Ok(())
    }

    fn with_data<T>(&self, work: impl FnOnce(&mut Data) -> T) -> T {
        let mut data = lock(&self.data).pop().unwrap_or_default();
        let result = work(&mut data);
        lock(&self.data).push(data);
        result
    }

    fn copy_data(
        &self,
        data: &mut Data,
        target_mount: &Mount,
        (source, source_stat, source_mount): (BorrowedFd<'_>, &EntryStat, Option<&Mount>),
        target: BorrowedFd<'_>,
        parent: &EntryStat,
        counted: bool,
    ) -> Result<Option<u64>, Failure> {
        let route = data::Route::new(
            self.fs,
            target_mount,
            source_mount,
            source,
            source_stat,
            parent,
        );
        let progress = (counted && self.counted).then_some(&*self.progress);
        data.copy(
            self.fs,
            &self.cancelled,
            target_mount,
            &route,
            (source, target),
            source_stat.size,
            |bytes| {
                if let Some(progress) = progress {
                    progress.add_bytes(bytes);
                }
            },
        )
    }

    fn copy_digested(
        &self,
        data: &mut Data,
        (source, source_stat): (BorrowedFd<'_>, &EntryStat),
        target: BorrowedFd<'_>,
        counted: bool,
    ) -> Result<(ring::digest::Digest, u64), Failure> {
        let progress = (counted && self.counted).then_some(&*self.progress);
        data.copy_digested(
            self.fs,
            &self.cancelled,
            (source, target),
            source_stat.size,
            |bytes| {
                if let Some(progress) = progress {
                    progress.add_bytes(bytes);
                }
            },
        )
    }

    /// When recording, a source that changed while it was read is read once
    /// more (`None` if it changed again), and one read within a tick of its
    /// mount's coarse times, or of the destination's, is digested for the
    /// move to compare.
    fn copy_file(
        &self,
        data: &mut Data,
        target_mount: &Mount,
        source: &Opened,
        target: BorrowedFd<'_>,
        parent: &EntryStat,
    ) -> Result<Option<(Snapshot, manifest::Read)>, Failure> {
        let mut stat = source.stat;
        let source_fd = source.fd.as_fd();
        // A folder may hold another mount, so each file asks its own.
        let source_mount = self.recording.then(|| Mount::new(self.fs, source_fd));
        for attempt in 0..2 {
            if attempt > 0 {
                self.fs.set_len(target, 0)?;
            }
            let counted = attempt == 0;
            let now = manifest::now();
            let unsettled = source_mount.and_then(|mount| manifest::unsettled(&stat, &mount, now));
            // The copy takes the source's modification time, which a file put
            // in its place within a tick of the destination's clock can share.
            let copy_unsettled =
                self.recording && manifest::unsettled(&stat, target_mount, now).is_some();
            let (digest, end) = if unsettled.is_some() || copy_unsettled {
                let (digest, end) =
                    self.copy_digested(data, (source_fd, &stat), target, counted)?;
                (Some(digest), Some(end))
            } else {
                let end = self.copy_data(
                    data,
                    target_mount,
                    (source_fd, &stat, source_mount.as_ref()),
                    target,
                    parent,
                    counted,
                )?;
                (None, end)
            };
            let snapshot = Snapshot::read(self.fs, source_fd, &stat)?;
            if !self.recording {
                let read = manifest::Read {
                    stat,
                    contents: None,
                    copied: None,
                };
                return Ok(Some((snapshot, read)));
            }
            let read = self.fs.stat_fd(source_fd)?;
            // A rewrite within one tick of a coarse clock keeps the times, not
            // always the size the copy read up to. Virtual files report none.
            let whole =
                end.is_none_or(|end| end == read.size || (stat.size == 0 && read.size == 0));
            if !manifest::changed(&stat, &read) && whole {
                let read = manifest::Read {
                    stat: read,
                    contents: digest
                        .zip(unsettled)
                        .map(|(digest, unsettled)| manifest::Contents::new(digest, unsettled)),
                    copied: digest,
                };
                return Ok(Some((snapshot, read)));
            }
            stat = read;
        }
        Ok(None)
    }

    fn apply_metadata(
        &self,
        fd: BorrowedFd<'_>,
        created: &EntryStat,
        snapshot: &Snapshot,
        parent: &EntryStat,
        mount: &Mount,
    ) -> Result<(), Failure> {
        snapshot.apply(self.fs, fd, created, parent, mount, &self.refused_xattrs)?;
        let stat = self.fs.stat_fd(fd)?;
        self.record_state(created.identity, &stat);
        Ok(())
    }

    fn record_state(&self, identity: LocalFileIdentity, stat: &EntryStat) {
        lock(&self.written).insert(
            (identity.device, identity.inode),
            (identity, manifest::State::of(stat)),
        );
    }

    fn written_attributes(&self, identity: LocalFileIdentity) -> Option<Attributes> {
        let written = lock(&self.written);
        let (known, state) = written.get(&(identity.device, identity.inode))?;
        (*known == identity).then(|| state.attributes())
    }

    fn settle_state(&self, identity: LocalFileIdentity, stat: &EntryStat) {
        if let Some((known, state)) =
            lock(&self.written).get_mut(&(identity.device, identity.inode))
            && *known == identity
        {
            state.settle(stat);
        }
    }

    /// A copy digested as it was written is compared by content before its
    /// source is deleted, as within a tick another file can take its place.
    fn wrote(&self, identity: LocalFileIdentity, read: &manifest::Read) {
        let Some(copied) = read.copied else {
            return;
        };
        if let Some((known, state)) =
            lock(&self.written).get_mut(&(identity.device, identity.inode))
            && *known == identity
        {
            *state = state.holding(copied);
        }
    }

    /// NFSv4 write delegations set the times at close, and FUSE daemons with
    /// a writeback cache and some SMB servers may too, so a closed file gets
    /// its times again: always on NFSv4, and where a check finds them lost.
    fn settle_times(
        &self,
        entry: Entry<'_>,
        identity: LocalFileIdentity,
        snapshot: &Snapshot,
        mount: &Mount,
    ) -> Result<(), Failure> {
        let lost = match mount.family {
            Family::Nfs4 => true,
            Family::Fuse | Family::Smb => match self.fs.stat(entry, true) {
                Ok(current) if snapshot.times_match(&current) => {
                    // Times moved by less than the check notices stay, so the
                    // copy is known by them from now on.
                    if mount.may_be(identity, &current) {
                        self.settle_state(identity, &current);
                    }
                    false
                }
                Ok(_) => true,
                Err(error) if is_fatal(error) => return Err(error.into()),
                Err(_) => false,
            },
            _ => false,
        };
        if !lost {
            return Ok(());
        }
        let fs = self.fs;
        let reopened = match fs.open_source(entry, false) {
            // A mode that gives the owner no read, as a copy of another
            // user's file readable through its group or others may have.
            Err(Errno::ACCESS) => fs.open_write(entry).and_then(|fd| {
                let stat = fs.stat_fd(fd.as_fd())?;
                Ok(Some((fd, stat)))
            }),
            opened => opened,
        };
        let reopened = match reopened {
            Ok(Some((fd, stat))) if mount.may_be(identity, &stat) => fd,
            // Setting times by name needs only ownership.
            Err(Errno::ACCESS) => {
                return match fs.stat(entry, true) {
                    Ok(current) if mount.may_be(identity, &current) => {
                        snapshot.apply_times_at(fs, entry)?;
                        if let Ok(settled) = fs.stat(entry, true)
                            && mount.may_be(identity, &settled)
                        {
                            self.settle_state(identity, &settled);
                        }
                        Ok(())
                    }
                    Err(error) if is_fatal(error) => Err(error.into()),
                    _ => Ok(()),
                };
            }
            Err(error) if is_fatal(error) => return Err(error.into()),
            _ => return Ok(()),
        };
        snapshot.apply_times(fs, reopened.as_fd())?;
        // The copy is known by the times it is left with.
        self.settle_state(identity, &fs.stat_fd(reopened.as_fd())?);
        match fs.close(reopened) {
            Err(error) if is_fatal(error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    fn discard(&self, staged: StagedSibling) -> Result<(), glib::Error> {
        unlock_stage(&staged);
        let identity = Some(staged.recorded());
        let roots = vec![PinnedEntry {
            parent: staged.parent,
            name: staged.name,
            identity,
        }];
        let workers = local_delete_worker_count(&roots);
        parallel_delete_local_blocking_with_workers(
            roots,
            Arc::new(AtomicBool::new(false)),
            workers,
            &self.leftovers,
        )
        .map_err(io_error)
    }

    fn discarding(&self, staged: StagedSibling, failure: Failure) -> Failure {
        let discarded = self.discard(staged);
        self.after_cleanup(failure, discarded)
    }

    fn after_cleanup(&self, failure: Failure, cleanup: Result<(), glib::Error>) -> Failure {
        match cleanup {
            Ok(()) => failure,
            Err(cleanup) => Failure::Error(copy_failure_after_cleanup(
                failure.into_error(&self.item),
                Err(cleanup),
            )),
        }
    }
}

struct Opened {
    fd: SourceFd,
    stat: EntryStat,
}

struct SourceFd {
    fd: Option<OwnedFd>,
    held: Option<OneOpen>,
}

impl SourceFd {
    fn new(fd: OwnedFd) -> Self {
        Self {
            fd: Some(fd),
            held: None,
        }
    }
}

impl AsFd for SourceFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_ref().expect("open until dropped").as_fd()
    }
}

impl Drop for SourceFd {
    fn drop(&mut self) {
        // Closed before `held` is released, as the fields drop after this.
        drop(self.fd.take());
    }
}

fn open_failure(name: &OsStr, error: Errno) -> Failure {
    match error {
        Errno::NOENT | Errno::NOTDIR | Errno::LOOP | Errno::STALE | Errno::NXIO | Errno::NODEV => {
            Failure::Error(io_error(rust_i18n::t!(
                "%{name} changed while it was being copied: %{error}",
                name = name.to_string_lossy(),
                error = os_error_detail(error)
            )))
        }
        error => Failure::Os(error).named(name),
    }
}

fn open_file(fs: &dyn FsOps, at: Entry<'_>, cancelled: &AtomicBool) -> Result<Opened, Failure> {
    let opened = match fs.open_source(at, true) {
        Err(Errno::INVAL) => open_after_deferred_close(fs, at, cancelled),
        opened => opened.map_err(|error| open_failure(at.name, error)),
    };
    match opened? {
        Some((fd, stat)) => Ok(Opened {
            fd: SourceFd::new(fd),
            stat,
        }),
        None => Err(Failure::Changed),
    }
}

/// After this machine writes a file, the SMB client keeps its handle open
/// for `closetimeo` and refuses an open of another name of the file with
/// `EINVAL` meanwhile, so the open is tried again until then.
fn open_after_deferred_close(
    fs: &dyn FsOps,
    at: Entry<'_>,
    cancelled: &AtomicBool,
) -> Result<Option<(OwnedFd, EntryStat)>, Failure> {
    let Some(deferred) = Mount::new(fs, at.dir).deferred_close else {
        return Err(open_failure(at.name, Errno::INVAL));
    };
    let mut wait = deferred_close::Wait::after(deferred);
    loop {
        match wait.pause(&|| cancelled.load(Ordering::Acquire)) {
            Pause::Retry => {}
            Pause::Spent => return Err(open_failure(at.name, Errno::INVAL)),
            Pause::Cancelled => return Err(Failure::Cancelled),
        }
        match fs.open_source(at, true) {
            Err(Errno::INVAL) => {}
            opened => return opened.map_err(|error| open_failure(at.name, error)),
        }
    }
}

fn open_one_name(
    fs: &dyn FsOps,
    at: Entry<'_>,
    seen: &EntryStat,
    cancelled: &AtomicBool,
) -> Result<Opened, Failure> {
    let held = OneOpen::wait(seen, cancelled)?;
    let mut source = open_file(fs, at, cancelled)?;
    source.fd.held = Some(held);
    if source.stat.identity != seen.identity {
        return Err(Failure::Changed);
    }
    Ok(source)
}

/// Files on SMB sources that a copy has a name of open, by device and
/// inode: the SMB client takes one lease per file and refuses an open of
/// another name meanwhile, or a Samba server defers it forever. Link counts
/// the client took from a listing say 1, so every file is held.
static OPEN_ON_SMB: Mutex<BTreeSet<(u64, u64)>> = Mutex::new(BTreeSet::new());
static OPEN_ON_SMB_CLOSED: Condvar = Condvar::new();

struct OneOpen((u64, u64));

impl OneOpen {
    fn wait(seen: &EntryStat, cancelled: &AtomicBool) -> Result<Self, Failure> {
        let key = (seen.identity.device, seen.identity.inode);
        let mut open = lock(&OPEN_ON_SMB);
        while !open.insert(key) {
            if cancelled.load(Ordering::Acquire) {
                return Err(Failure::Cancelled);
            }
            open = OPEN_ON_SMB_CLOSED
                .wait_timeout(open, Duration::from_millis(50))
                .map_or_else(|error| error.into_inner().0, |(open, _)| open);
        }
        Ok(Self(key))
    }
}

impl Drop for OneOpen {
    fn drop(&mut self) {
        lock(&OPEN_ON_SMB).remove(&self.0);
        OPEN_ON_SMB_CLOSED.notify_all();
    }
}

#[derive(Clone, Copy)]
enum FileTarget<'a> {
    Final(&'a OsStr, Taken),
    Hidden,
}

struct Stage {
    name: OsString,
    created: EntryStat,
}

struct FileCopy<'a> {
    copy: &'a Copy,
    dir: BorrowedFd<'a>,
    target: Mount,
    data: Data,
    read: Option<manifest::Read>,
}

impl<'a> FileCopy<'a> {
    fn new(copy: &'a Copy, dir: BorrowedFd<'a>) -> Self {
        Self {
            copy,
            dir,
            target: Mount::new(copy.fs, dir),
            data: Data::default(),
            read: None,
        }
    }

    fn at<'b>(&self, name: &'b OsStr) -> Entry<'b>
    where
        'a: 'b,
    {
        Entry {
            dir: self.dir,
            name,
        }
    }

    fn open(&self, at: Entry<'_>, seen: &EntryStat) -> Result<Opened, Failure> {
        let fs = self.copy.fs;
        if Mount::new(fs, at.dir).family != Family::Smb {
            return open_file(fs, at, &self.copy.cancelled);
        }
        open_one_name(fs, at, seen, &self.copy.cancelled)
    }

    fn copy_from(
        &mut self,
        at: Entry<'_>,
        seen: &EntryStat,
        target: FileTarget<'_>,
        recorded_in: Option<&manifest::Folder>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let fs = self.copy.fs;
        self.copy.check_cancelled()?;
        let source = self.open(at, seen)?;
        let parent = fs.stat_fd(self.dir)?;
        let mut tmpfile = None;
        if stage::tmpfile_allowed(self.target.family)
            && self.target.capabilities().tmpfile != Some(false)
        {
            match fs.open_tmpfile(self.dir, Mode::from_raw_mode(0o600)) {
                Ok(fd) => {
                    self.target.learn(|capabilities| {
                        capabilities.tmpfile.get_or_insert(true);
                    });
                    tmpfile = Some(fd);
                }
                // Before Linux 3.11 the flag reads as O_DIRECTORY.
                Err(error) if is_unsupported(error) || error == Errno::ISDIR => {
                    self.target
                        .learn(|capabilities| capabilities.tmpfile = Some(false));
                }
                Err(error) => return Err(error.into()),
            }
        }
        let copied = match tmpfile {
            Some(fd) => self.through_tmpfile(&source, fd, &parent, target),
            None => self.through_named_stage(&source, &parent, target),
        }?;
        if let (Some(folder), Some(read)) = (recorded_in, self.read.take()) {
            self.copy.wrote(copied.1, &read);
            folder.file(at.name, read);
        }
        Ok(copied)
    }

    fn in_place(
        &mut self,
        at: Entry<'_>,
        seen: &EntryStat,
        name: &OsStr,
        recorded_in: Option<&manifest::Folder>,
    ) -> Result<LocalFileIdentity, Failure> {
        let fs = self.copy.fs;
        self.copy.check_cancelled()?;
        let source = self.open(at, seen)?;
        if source.stat.identity != seen.identity {
            return Err(Failure::Changed);
        }
        let parent = fs.stat_fd(self.dir)?;
        let identity = self.create(name, &parent, |file, fd| {
            file.read_into(&source, fd, &parent)
        })?;
        if let (Some(folder), Some(read)) = (recorded_in, self.read.take()) {
            self.copy.wrote(identity, &read);
            folder.file(at.name, read);
        }
        self.copy.add_file(0);
        Ok(identity)
    }

    fn read_into(
        &mut self,
        source: &Opened,
        target: BorrowedFd<'_>,
        parent: &EntryStat,
    ) -> Result<Snapshot, Failure> {
        let (snapshot, read) = self
            .copy
            .copy_file(&mut self.data, &self.target, source, target, parent)?
            .ok_or(Failure::Changed)?;
        self.read = Some(read);
        Ok(snapshot)
    }

    fn link(&self, tmpfile: BorrowedFd<'_>, target: FileTarget<'_>) -> Result<OsString, Errno> {
        let fs = self.copy.fs;
        match target {
            FileTarget::Final(name, taken) => {
                first_free(self.copy, self.dir, (name, taken, false), |name| {
                    let to = self.at(name);
                    link_tmpfile(fs, &self.target, tmpfile, to).map_err(|error| {
                        match placement::file_creation_error(error) {
                            PlaceError::Exists => Failure::Place(PlaceError::Exists),
                            _ => creation_failure(fs, to, error),
                        }
                    })
                })
                .map(|(name, ())| name)
                .map_err(|failure| match failure {
                    Failure::Os(error) | Failure::NameRefused(error) => error,
                    _ => Errno::EXIST,
                })
            }
            FileTarget::Hidden => placement::reserve_hidden_name(HiddenName::Copy, |name| {
                link_tmpfile(fs, &self.target, tmpfile, self.at(name)).map_err(|error| {
                    match placement::file_creation_error(error) {
                        PlaceError::Exists => PlaceError::Exists,
                        _ => PlaceError::Io(error),
                    }
                })
            })
            .map(|(name, ())| name)
            .map_err(|error| match error {
                PlaceError::Io(error) => error,
                _ => Errno::EXIST,
            }),
        }
    }

    fn through_tmpfile(
        &mut self,
        source: &Opened,
        tmpfile: OwnedFd,
        parent: &EntryStat,
        target: FileTarget<'_>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let fs = self.copy.fs;
        let snapshot = self.read_into(source, tmpfile.as_fd(), parent)?;
        let created = fs.stat_fd(tmpfile.as_fd())?;
        self.copy
            .apply_metadata(tmpfile.as_fd(), &created, &snapshot, parent, &self.target)?;
        self.copy.sync_written(tmpfile.as_fd(), &self.target)?;
        self.copy.check_cancelled()?;
        let name = match self.link(tmpfile.as_fd(), target) {
            Ok(name) => name,
            Err(error) => {
                return match placement::file_creation_error(error) {
                    PlaceError::Exists => Err(Failure::Place(PlaceError::Exists)),
                    _ if matches!(error, Errno::NOSPC | Errno::DQUOT) || is_fatal(error) => {
                        Err(error.into())
                    }
                    // A stage would be refused that name too; `ENOENT` may
                    // instead mean that unnamed files cannot be linked.
                    _ if matches!(
                        error,
                        Errno::INVAL | Errno::ILSEQ | Errno::NAMETOOLONG | Errno::BADMSG
                    ) =>
                    {
                        Err(Failure::NameRefused(error))
                    }
                    // The written bytes are copied out on the same device, so
                    // the source is never read twice.
                    _ => self
                        .through_stage_from_tmpfile(tmpfile, &created, &snapshot, parent, target),
                };
            }
        };
        if let Err(error) = fs.close(tmpfile) {
            placement::remove_if_unchanged(
                fs,
                self.at(&name),
                created.identity,
                self.copy.written_attributes(created.identity),
                AtFlags::empty(),
            );
            return Err(error.into());
        }
        self.copy.add_file(0);
        Ok((name, created.identity))
    }

    fn through_stage_from_tmpfile(
        &mut self,
        tmpfile: OwnedFd,
        tmpfile_stat: &EntryStat,
        snapshot: &Snapshot,
        parent: &EntryStat,
        target: FileTarget<'_>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let (fd, stage) = self.create_stage()?;
        let copied = self
            .copy
            .copy_data(
                &mut self.data,
                &self.target,
                (tmpfile.as_fd(), tmpfile_stat, Some(&self.target)),
                fd.as_fd(),
                parent,
                false,
            )
            .and_then(|_| {
                self.copy
                    .apply_metadata(fd.as_fd(), &stage.created, snapshot, parent, &self.target)
            });
        drop(tmpfile);
        match copied {
            Ok(()) => self.finish_stage(fd, stage, snapshot, parent, target),
            Err(failure) => {
                drop(fd);
                Err(self.discard(&stage, failure))
            }
        }
    }

    fn through_named_stage(
        &mut self,
        source: &Opened,
        parent: &EntryStat,
        target: FileTarget<'_>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let (fd, stage) = self.create_stage()?;
        let copied = self
            .read_into(source, fd.as_fd(), parent)
            .and_then(|snapshot| {
                self.copy.apply_metadata(
                    fd.as_fd(),
                    &stage.created,
                    &snapshot,
                    parent,
                    &self.target,
                )?;
                Ok(snapshot)
            });
        match copied {
            Ok(snapshot) => self.finish_stage(fd, stage, &snapshot, parent, target),
            Err(failure) => {
                drop(fd);
                Err(self.discard(&stage, failure))
            }
        }
    }

    fn create_stage(&self) -> Result<(OwnedFd, Stage), Failure> {
        let (name, (fd, created)) = stage::create_file(self.copy.fs, self.dir)?;
        Ok((fd, Stage { name, created }))
    }

    fn finish_stage(
        &mut self,
        fd: OwnedFd,
        stage: Stage,
        snapshot: &Snapshot,
        parent: &EntryStat,
        target: FileTarget<'_>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let fs = self.copy.fs;
        let _pinned = Pinned::at(
            fs,
            self.at(&stage.name),
            &stage.created,
            self.target.identity,
        );
        let settled = self
            .copy
            .sync_written(fd.as_fd(), &self.target)
            .and_then(|()| fs.close(fd).map_err(Failure::Os))
            .and_then(|()| {
                self.copy.settle_times(
                    self.at(&stage.name),
                    stage.created.identity,
                    snapshot,
                    &self.target,
                )
            })
            .and_then(|()| self.copy.check_cancelled());
        if let Err(failure) = settled {
            return Err(self.discard(&stage, failure));
        }
        let FileTarget::Final(name, taken) = target else {
            self.copy.add_file(0);
            return Ok((stage.name, stage.created.identity));
        };
        let (copy, dir) = (self.copy, self.dir);
        let staged = Entry {
            dir,
            name: &stage.name,
        };
        let mut created_from = false;
        let published = first_free(copy, dir, (name, taken, false), |name| {
            if !created_from {
                match stage::publish(fs, dir, &stage.name, stage.created.identity, name) {
                    Err(PlaceError::Unsupported) => created_from = true,
                    published => {
                        return published
                            .map(|_| stage.created.identity)
                            .map_err(Failure::Place);
                    }
                }
            }
            self.create_from(staged, stage.created.identity, Some(snapshot), name, parent)
        });
        let published = match published {
            Ok(published) => published,
            // Either name may hold the copy, so neither is removed.
            Err(Failure::Place(PlaceError::Unconfirmed)) if !created_from => {
                return Err(Failure::Place(PlaceError::Unconfirmed));
            }
            Err(failure) => return Err(self.discard(&stage, failure)),
        };
        if created_from && let Err(error) = self.remove_made(&stage.name, stage.created.identity) {
            return Err(Failure::Error(stage_left(&error)));
        }
        self.copy.add_file(0);
        Ok(published)
    }

    fn create_from(
        &mut self,
        from: Entry<'_>,
        expected: LocalFileIdentity,
        snapshot: Option<&Snapshot>,
        name: &OsStr,
        parent: &EntryStat,
    ) -> Result<LocalFileIdentity, Failure> {
        let fs = self.copy.fs;
        let source = match fs.open_source(from, false)? {
            Some((fd, stat)) if stat.identity == expected => Opened {
                fd: SourceFd::new(fd),
                stat,
            },
            _ => return Err(Failure::Changed),
        };
        let read;
        let snapshot = match snapshot {
            Some(snapshot) => snapshot,
            None => {
                read = Snapshot::read(fs, source.fd.as_fd(), &source.stat)?;
                &read
            }
        };
        self.create(name, parent, |file, fd| {
            let from = (source.fd.as_fd(), &source.stat, None);
            file.copy
                .copy_data(&mut file.data, &file.target, from, fd, parent, false)?;
            Ok(snapshot)
        })
    }

    fn create<S: Borrow<Snapshot>>(
        &mut self,
        name: &OsStr,
        parent: &EntryStat,
        write: impl FnOnce(&mut Self, BorrowedFd<'_>) -> Result<S, Failure>,
    ) -> Result<LocalFileIdentity, Failure> {
        let fs = self.copy.fs;
        let target = self.at(name);
        let fd = fs
            .create_exclusive(target, Mode::from_raw_mode(0o600))
            .map_err(|error| match creation_failure(fs, target, error) {
                Failure::Os(error) => Failure::Place(placement::file_creation_error(error)),
                refused => refused,
            })?;
        let created = fs.stat_fd(fd.as_fd())?;
        // Takes `fd`, so a failed file is closed before it is removed: NFS
        // renames an open file aside instead.
        let filled = (|fd: OwnedFd| -> Result<(), Failure> {
            let written = write(self, fd.as_fd())?;
            let snapshot: &Snapshot = written.borrow();
            self.copy
                .apply_metadata(fd.as_fd(), &created, snapshot, parent, &self.target)?;
            self.copy.sync_written(fd.as_fd(), &self.target)?;
            fs.close(fd)?;
            self.copy
                .settle_times(target, created.identity, snapshot, &self.target)
        })(fd);
        match filled {
            Ok(()) => Ok(created.identity),
            Err(failure) => {
                let removed = self.remove_made(name, created.identity);
                Err(self.copy.after_cleanup(failure, removed))
            }
        }
    }

    fn remove_made(&self, name: &OsStr, identity: LocalFileIdentity) -> Result<(), glib::Error> {
        let fs = self.copy.fs;
        remove_own(
            fs,
            self.at(name),
            |current| current.identity == identity,
            |at| {
                fs.unlink(at, AtFlags::empty())
                    .map_err(leftovers::Refused::Os)
            },
        )
    }

    fn discard(&self, stage: &Stage, failure: Failure) -> Failure {
        let removed = self.remove_made(&stage.name, stage.created.identity);
        self.copy.after_cleanup(failure, removed)
    }
}
