// SPDX-License-Identifier: MIT

//! What a move copied of each source entry, as it was once the copy had
//! read it, so that afterwards only the entries the copy holds unchanged
//! are deleted. It keeps names, never descriptors. What an operation placed
//! is recorded the same way, for undo to delete only while unchanged.

use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gtk::glib;
use ring::digest;
use rustix::{
    fs::{AtFlags, FileType},
    io::Errno,
};

use super::{Failure, Mount, lock, restrictive};
use crate::{
    adapters::{
        local_operations::{
            cancelled_local_operation,
            deletion::bounded_local_worker_count,
            fs_ops::{Entry, EntryStat, FsOps, OffMain},
            io_error,
            leftovers::{self, Deadline, Refused},
            native_fs::{self, LocalFileIdentity},
            translated_io_error,
            work_queue::{self, Group, Job, WorkQueue},
        },
        volume::{CoarseTimes, Family},
    },
    services::{Attributes, IdentityTraits, InodeNumbers, KeptCause, Proof},
};

/// The link count each name deletions removed of a file with several names
/// had just before, by device and inode. A transfer's deletions share one,
/// as its items may hold names of one file. Those counts tell which
/// removals followed a snapshot: the ones at or below its count.
#[derive(Default)]
pub(in crate::adapters::local_operations) struct Unlinks {
    unlinked: Mutex<HashMap<(u64, u64), Unlinked>>,
    leftovers: Deadline,
}

impl Unlinks {
    pub(in crate::adapters::local_operations) fn leftovers(&self) -> &Deadline {
        &self.leftovers
    }
}

#[derive(Default)]
struct Unlinked {
    counts: Vec<u64>,
    left: Option<(i64, u32)>,
}

impl Unlinked {
    fn since(&self, nlink: u64) -> (u64, Option<(i64, u32)>) {
        let since = self.counts.iter().filter(|&&count| count <= nlink).count() as u64;
        let latest_followed = self.counts.last().is_some_and(|&count| count <= nlink);
        (since, self.left.filter(|_| latest_followed))
    }
}

/// The time tests pretend it is, which waits move on instead of sleeping.
#[cfg(test)]
static TEST_NOW: Mutex<Option<SystemTime>> = Mutex::new(None);

#[cfg(test)]
pub(in crate::adapters::local_operations) fn set_clock_for_test(now: Option<SystemTime>) {
    *lock(&TEST_NOW) = now;
}

pub(in crate::adapters::local_operations) fn now() -> SystemTime {
    #[cfg(test)]
    if let Some(now) = *lock(&TEST_NOW) {
        return now;
    }
    SystemTime::now()
}

pub(in crate::adapters::local_operations) fn sleep(duration: Duration) {
    #[cfg(test)]
    if let Some(pretended) = lock(&TEST_NOW).as_mut() {
        *pretended += duration;
        return;
    }
    thread::sleep(duration);
}

enum Waited {
    Settled,
    Stopped,
    /// The clock went back to before the copy read the source: a rewrite
    /// may then be stamped within the tick the copy read, and waiting would
    /// last as long as it went back.
    SetBack,
}

fn wait_until(unsettled: Unsettled, stopped: &dyn Fn() -> bool) -> Waited {
    loop {
        let now = now();
        if now < unsettled.read {
            return Waited::SetBack;
        }
        let left = unsettled.settled.duration_since(now).unwrap_or_default();
        if left.is_zero() {
            return Waited::Settled;
        }
        if stopped() {
            return Waited::Stopped;
        }
        #[cfg(test)]
        if let Some(pretended) = lock(&TEST_NOW).as_mut() {
            *pretended = unsettled.settled;
            continue;
        }
        thread::sleep(left.min(Duration::from_millis(50)));
    }
}

/// How far the coarse clock filesystems stamp times with may run behind
/// ours: a kernel tick, at most 10 ms, with room to spare.
const COARSE_CLOCK: Duration = Duration::from_millis(100);

#[derive(Clone, Copy)]
pub(super) struct Unsettled {
    read: SystemTime,
    settled: SystemTime,
    /// A modification time ahead of our clock, which stamps it: until that
    /// clock reaches it, no rewrite can have left it as it is.
    ahead: Option<SystemTime>,
}

pub(super) fn unsettled(stat: &EntryStat, mount: &Mount, started: SystemTime) -> Option<Unsettled> {
    let (seconds, nanoseconds) = stat.mtime;
    let coarse = mount
        .coarse_times
        .or_else(|| CoarseTimes::whole_second(nanoseconds))
        .or_else(|| mount.family.coarse_clock(super::data::kernel()))?;
    let modified =
        UNIX_EPOCH.checked_add(Duration::new(u64::try_from(seconds).ok()?, nanoseconds))?;
    // Our clock stamps a time ahead of it only once it reaches that time; a
    // server's, which may run ahead of ours, may stamp it now.
    let stamped = if coarse.stamped_here {
        modified
    } else {
        modified.min(started)
    };
    let settled = stamped.checked_add(coarse.window + COARSE_CLOCK)?;
    (started < settled).then_some(Unsettled {
        read: started,
        settled,
        ahead: (coarse.stamped_here && started < modified).then_some(modified),
    })
}

#[derive(Clone)]
pub(super) struct Contents {
    digest: digest::Digest,
    unsettled: Unsettled,
    /// How many leading bytes the digest covers, if not all of them.
    prefix: Option<u64>,
}

impl Contents {
    pub(super) fn new(digest: digest::Digest, unsettled: Unsettled) -> Self {
        Self {
            digest,
            unsettled,
            prefix: None,
        }
    }
}

pub(super) struct Read {
    pub stat: EntryStat,
    pub contents: Option<Contents>,
    /// What the copy holds, where its times cannot tell another file put in
    /// its place.
    pub copied: Option<digest::Digest>,
}

const MIB: usize = 1 << 20;

#[derive(Clone)]
struct SourceSnapshot {
    identity: LocalFileIdentity,
    nlink: u64,
    size: u64,
    mtime: (i64, u32),
    ctime: (i64, u32),
    contents: Option<Box<Contents>>,
}

impl SourceSnapshot {
    fn of(stat: &EntryStat, contents: Option<Contents>) -> Self {
        Self {
            identity: stat.identity,
            nlink: stat.nlink,
            size: stat.size,
            mtime: stat.mtime,
            ctime: stat.ctime,
            contents: contents.map(Box::new),
        }
    }

    /// Removing a name changes the ctime and link count of the others.
    fn holds(
        &self,
        current: &EntryStat,
        traits: IdentityTraits,
        (unlinked, left): (u64, Option<(i64, u32)>),
    ) -> bool {
        let recorded = Attributes {
            size: self.size,
            mtime: self.mtime,
            ctime: None,
        };
        let same = native_fs::compare(traits, self.identity, Some(recorded), current)
            == Proof::Same
            && (current.size, current.mtime) == (self.size, self.mtime);
        if !counts_links(traits) {
            // Nothing tells which names of a file this deletion removed. Where
            // times are as coarse as FAT's, only a birth time tells a file put
            // in its place apart.
            return same && (current.ctime == self.ctime || self.nlink > 1);
        }
        // Every change to an entry moves its change time; the identity and
        // link count are defence in depth.
        same && current.nlink + unlinked == self.nlink
            && match unlinked {
                0 => current.ctime == self.ctime,
                _ if traits.numbers == InodeNumbers::Renewed => left == Some(current.ctime),
                _ => left.is_none_or(|left| current.ctime == left),
            }
    }
}

fn counts_links(traits: IdentityTraits) -> bool {
    traits.numbers != InodeNumbers::Renewed || traits.made_up_births
}

pub(super) fn changed(before: &EntryStat, after: &EntryStat) -> bool {
    before.size != after.size || before.mtime != after.mtime || before.ctime != after.ctime
}

pub(super) struct Folder {
    /// `None` for the folder holding the moved item, which stays.
    identity: Option<LocalFileIdentity>,
    children: Mutex<Vec<(OsString, Node)>>,
}

enum Node {
    Entry(SourceSnapshot),
    Folder(Arc<Folder>),
    Kept(KeptCause),
}

impl Node {
    fn copied(&self) -> Self {
        match self {
            Self::Entry(snapshot) => Self::Entry(snapshot.clone()),
            Self::Folder(folder) => Self::Folder(Arc::new(Folder {
                identity: folder.identity,
                children: Mutex::new(
                    lock(&folder.children)
                        .iter()
                        .map(|(name, node)| (name.clone(), node.copied()))
                        .collect(),
                ),
            })),
            Self::Kept(cause) => Self::Kept(cause.clone()),
        }
    }
}

impl Folder {
    pub(super) fn root() -> Arc<Self> {
        Arc::new(Self {
            identity: None,
            children: Mutex::default(),
        })
    }

    pub(super) fn folder(&self, name: &OsStr, identity: LocalFileIdentity) -> Arc<Self> {
        let folder = Arc::new(Self {
            identity: Some(identity),
            children: Mutex::default(),
        });
        self.add(name, Node::Folder(folder.clone()));
        folder
    }

    pub(super) fn entry(&self, name: &OsStr, stat: &EntryStat) {
        self.add(name, Node::Entry(SourceSnapshot::of(stat, None)));
    }

    pub(super) fn file(&self, name: &OsStr, read: Read) {
        self.add(
            name,
            Node::Entry(SourceSnapshot::of(&read.stat, read.contents)),
        );
    }

    pub(super) fn keep(&self, name: &OsStr, cause: KeptCause) {
        self.add(name, Node::Kept(cause));
    }

    fn add(&self, name: &OsStr, node: Node) {
        lock(&self.children).push((name.to_os_string(), node));
    }

    fn take(&self) -> Vec<(OsString, Node)> {
        std::mem::take(&mut *lock(&self.children))
    }
}

pub(super) struct Pending {
    folder: Arc<Folder>,
    name: OsString,
    snapshot: SourceSnapshot,
    /// Whether the deletion may open this name to compare its contents.
    openable: bool,
}

impl Pending {
    pub(super) fn new(folder: Arc<Folder>, name: &OsStr, stat: &EntryStat, openable: bool) -> Self {
        Self {
            folder,
            name: name.to_os_string(),
            snapshot: SourceSnapshot::of(stat, None),
            openable,
        }
    }

    /// `contents` are the first name's: a later name is never read, and
    /// its times cannot tell a same-size rewrite within the tick.
    pub(super) fn made(mut self, contents: Option<Contents>) {
        if contents.is_some() && !self.openable {
            return self.keep(KeptCause::Changed);
        }
        self.snapshot.contents = contents.map(Box::new);
        self.folder.add(&self.name, Node::Entry(self.snapshot));
    }

    pub(super) fn keep(self, cause: KeptCause) {
        self.folder.keep(&self.name, cause);
    }
}

pub(in crate::adapters::local_operations) struct Manifest {
    pub(super) parent: OffMain<Arc<OwnedFd>>,
    pub(super) root: Arc<Folder>,
    copies: Vec<Published>,
}

impl Manifest {
    pub(in crate::adapters::local_operations) fn copied_to(&mut self, copy: Published) {
        self.copies.push(copy);
    }

    /// `from` and `to` are beneath the copy's root.
    pub(in crate::adapters::local_operations) fn follow_rename(&mut self, from: &Path, to: &Path) {
        for copy in &mut self.copies {
            copy.ownership.renamed(from, to);
        }
    }

    pub(in crate::adapters::local_operations) fn keep(&self, path: &Path, cause: KeptCause) {
        let mut folder = self.root.clone();
        let mut names = path.iter().peekable();
        while let Some(name) = names.next() {
            let mut children = lock(&folder.children);
            let Some((_, node)) = children.iter_mut().find(|(child, _)| child == name) else {
                return;
            };
            if names.peek().is_none() {
                *node = Node::Kept(cause);
                return;
            }
            let Node::Folder(next) = node else {
                return;
            };
            let next = next.clone();
            drop(children);
            folder = next;
        }
    }
}

/// A link's size is the length of its target.
fn has_state(kind: FileType) -> bool {
    matches!(kind, FileType::RegularFile | FileType::Symlink)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct State {
    size: u64,
    mtime: (i64, u32),
    /// What the copy wrote, where its times cannot tell another file put in
    /// its place.
    contents: Option<digest::Digest>,
}

impl State {
    pub(super) fn of(stat: &EntryStat) -> Self {
        Self {
            size: stat.size,
            mtime: stat.mtime,
            contents: None,
        }
    }

    pub(super) fn holding(self, contents: digest::Digest) -> Self {
        Self {
            contents: Some(contents),
            ..self
        }
    }

    fn holds(&self, stat: &EntryStat) -> bool {
        self.size == stat.size && self.mtime == stat.mtime
    }

    pub(super) fn attributes(&self) -> Attributes {
        Attributes {
            size: self.size,
            mtime: self.mtime,
            ctime: None,
        }
    }

    /// Where only the type tells the copy, a file of another size put in its
    /// place keeps the state the copy was written with.
    pub(super) fn settle(&mut self, stat: &EntryStat) {
        if self.size == stat.size {
            self.mtime = stat.mtime;
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::adapters::local_operations) struct Ownership {
    identity: Option<LocalFileIdentity>,
    descendants: HashMap<PathBuf, LocalFileIdentity>,
    states: HashMap<PathBuf, State>,
}

impl Ownership {
    pub(super) fn new(identity: LocalFileIdentity) -> Self {
        Self {
            identity: Some(identity),
            descendants: HashMap::new(),
            states: HashMap::new(),
        }
    }

    pub(super) fn unproven() -> Self {
        Self {
            identity: None,
            descendants: HashMap::new(),
            states: HashMap::new(),
        }
    }

    pub(super) fn created(
        identity: LocalFileIdentity,
        descendants: impl IntoIterator<Item = (PathBuf, LocalFileIdentity)>,
    ) -> Self {
        Self {
            identity: Some(identity),
            descendants: descendants.into_iter().collect(),
            states: HashMap::new(),
        }
    }

    pub(super) fn with_states(
        mut self,
        states: impl IntoIterator<Item = (PathBuf, State)>,
    ) -> Self {
        self.states.extend(states);
        self
    }

    pub(in crate::adapters::local_operations) fn attributes(&self) -> Option<Attributes> {
        self.states.get(Path::new("")).map(State::attributes)
    }

    pub(super) fn of(stat: &EntryStat) -> Self {
        let mut owned = Self::new(stat.identity);
        if has_state(stat.kind) {
            owned.states.insert(PathBuf::new(), State::of(stat));
        }
        owned
    }

    pub(super) fn capture(
        fs: &dyn FsOps,
        at: Entry<'_>,
        identity: LocalFileIdentity,
    ) -> Result<Self, Failure> {
        let stat = fs.stat(at, false)?;
        if stat.identity != identity {
            return Err(Failure::Changed);
        }
        let mut ownership = Self::of(&stat);
        if identity.kind == FileType::Directory {
            let dir = fs.open_dir(at)?;
            if fs.stat_fd(dir.as_fd())?.identity != identity {
                return Err(Failure::Changed);
            }
            ownership.read_children(fs, dir.as_fd(), Path::new(""))?;
        }
        Ok(ownership)
    }

    fn read_children(
        &mut self,
        fs: &dyn FsOps,
        dir: BorrowedFd<'_>,
        path: &Path,
    ) -> Result<(), Failure> {
        for (name, _) in fs.list(dir)? {
            let at = Entry { dir, name: &name };
            let stat = fs.stat(at, false)?;
            let path = path.join(&name);
            self.descendants.insert(path.clone(), stat.identity);
            if has_state(stat.kind) {
                self.states.insert(path.clone(), State::of(&stat));
            }
            if stat.is_directory() {
                let child = fs.open_dir(at)?;
                if fs.stat_fd(child.as_fd())?.identity != stat.identity {
                    return Err(Failure::Changed);
                }
                self.read_children(fs, child.as_fd(), &path)?;
            }
        }
        Ok(())
    }

    /// An entry the operation renamed afterwards keeps its identity under its new name.
    fn renamed(&mut self, from: &Path, to: &Path) {
        let rebase = |path: &PathBuf| crate::model::rebase_path(path, from, to);
        self.descendants = std::mem::take(&mut self.descendants)
            .into_iter()
            .map(|(path, identity)| (rebase(&path).unwrap_or(path), identity))
            .collect();
        self.states = std::mem::take(&mut self.states)
            .into_iter()
            .map(|(path, state)| (rebase(&path).unwrap_or(path), state))
            .collect();
    }

    fn identity_at(&self, path: &Path) -> Option<LocalFileIdentity> {
        if path.as_os_str().is_empty() {
            self.identity
        } else {
            self.descendants.get(path).copied()
        }
    }

    fn matches(&self, path: &Path, stat: &EntryStat) -> bool {
        self.identity_at(path)
            .is_some_and(|identity| stat.identity == identity)
    }

    fn unchanged(&self, path: &Path, stat: &EntryStat) -> bool {
        self.matches(path, stat)
            && (stat.kind != FileType::RegularFile
                || self.states.get(path).is_some_and(|state| state.holds(stat)))
    }

    /// A file rewritten in place, or another one given its inode number and
    /// birth time, shows only in its size or modification time, and within
    /// one tick of a coarse clock only in its contents.
    fn holds(
        &self,
        fs: &dyn FsOps,
        (at, path): (Entry<'_>, &Path),
        stat: &EntryStat,
        traits: IdentityTraits,
    ) -> bool {
        let Some(identity) = self.identity_at(path) else {
            return false;
        };
        let state = self.states.get(path);
        let exact = stat.identity == identity;
        let same = if has_state(identity.kind) && state.is_none() {
            exact
        } else {
            let attributes = native_fs::beside_birth(identity, state.map(State::attributes));
            match native_fs::compare(traits, identity, attributes, stat) {
                Proof::Same => true,
                Proof::Different => false,
                Proof::Unproven => exact || native_fs::birth_reset_folder(traits, identity, stat),
            }
        };
        same && state.is_none_or(|state| {
            state.holds(stat)
                && state
                    .contents
                    .is_none_or(|contents| holds_contents(fs, at, stat, contents))
        })
    }

    pub(in crate::adapters::local_operations) fn discard(
        &self,
        fs: &dyn FsOps,
        at: Entry<'_>,
    ) -> bool {
        self.identity
            .is_some_and(|identity| identity.kind != FileType::Directory)
            && self.stays(fs, at)
            && matches!(fs.unlink(at, AtFlags::empty()), Ok(()) | Err(Errno::NOENT))
    }

    pub(super) fn stays(&self, fs: &dyn FsOps, at: Entry<'_>) -> bool {
        let Ok(stat) = fs.stat(at, true) else {
            return false;
        };
        let traits = Mount::new(fs, at.dir).identity_now(fs, at.dir);
        if !self.holds(fs, (at, Path::new("")), &stat, traits) {
            return false;
        }
        if self.descendants.is_empty() {
            return true;
        }
        let Ok(root) = fs.open_dir(at) else {
            return false;
        };
        if !fs
            .stat_fd(root.as_fd())
            .is_ok_and(|opened| opened.identity == stat.identity)
        {
            return false;
        }
        self.descendants.iter().all(|(path, _)| {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                return false;
            };
            let Ok(dir) = fs.open_dir(Entry::new(&root, super::restrictive::beneath(parent)))
            else {
                return false;
            };
            let at = Entry::new(&dir, name);
            fs.stat(at, true)
                .is_ok_and(|stat| self.holds(fs, (at, path), &stat, traits))
        })
    }
}

/// Whether the file `stat` saw at `at` holds exactly the bytes digested, up to
/// one byte past its size.
fn holds_contents(
    fs: &dyn FsOps,
    at: Entry<'_>,
    stat: &EntryStat,
    contents: digest::Digest,
) -> bool {
    let Ok(Some((file, opened))) = fs.open_source(at, false) else {
        return false;
    };
    opened.identity == stat.identity
        && digest_of(fs, file.as_fd(), stat.size.saturating_add(1), &|| false)
            .is_ok_and(|(digest, prefix)| prefix.is_none() && digest.as_ref() == contents.as_ref())
}

pub(in crate::adapters::local_operations) struct Published {
    parent: OffMain<Arc<OwnedFd>>,
    name: OsString,
    ownership: Ownership,
}

impl Published {
    pub(super) fn new(parent: Arc<OwnedFd>, name: OsString, identity: LocalFileIdentity) -> Self {
        Self::proven(parent, name, Ownership::new(identity))
    }

    pub(super) fn proven(parent: Arc<OwnedFd>, name: OsString, ownership: Ownership) -> Self {
        Self {
            parent: OffMain::new(parent),
            name,
            ownership,
        }
    }

    pub(super) fn ownership(&self) -> &Ownership {
        &self.ownership
    }

    fn stays(&self, fs: &dyn FsOps) -> bool {
        self.ownership
            .stays(fs, Entry::new(&**self.parent, &self.name))
    }
}

pub(super) fn snapshot(
    fs: &dyn FsOps,
    parent: Arc<OwnedFd>,
    name: &OsStr,
    cancelled: &AtomicBool,
) -> Result<Manifest, Failure> {
    let root = Folder::root();
    let listing = Listing::of(fs, parent.as_fd(), false);
    record(fs, listing, name, &root, cancelled, None, None)?;
    Ok(Manifest {
        parent: OffMain::new(parent),
        root,
        copies: Vec::new(),
    })
}

pub(in crate::adapters::local_operations) struct Placed {
    name: OsString,
    node: Node,
}

impl Placed {
    /// A fresh record to delete by, as deleting takes what it deletes.
    pub(super) fn root(&self) -> Arc<Folder> {
        let root = Folder::root();
        root.add(&self.name, self.node.copied());
        root
    }
}

/// The most entries an operation records of what it placed, at some 200
/// bytes each, as undo history keeps them.
const PLACED_ENTRIES: usize = 100_000;

#[cfg(test)]
static TEST_PLACED_ENTRIES: Mutex<Option<usize>> = Mutex::new(None);

#[cfg(test)]
pub(in crate::adapters::local_operations) fn set_placed_entries_for_test(limit: Option<usize>) {
    *lock(&TEST_PLACED_ENTRIES) = limit;
}

pub(super) fn placed_entries() -> AtomicUsize {
    #[cfg(test)]
    if let Some(limit) = *lock(&TEST_PLACED_ENTRIES) {
        return AtomicUsize::new(limit);
    }
    AtomicUsize::new(PLACED_ENTRIES)
}

/// Run right after placing. Fails once the operation's records would hold more than `left`
/// entries, and gives back what it took.
#[cfg(test)]
pub(super) fn placed(
    fs: &dyn FsOps,
    (dir, name): (BorrowedFd<'_>, &OsStr),
    cancelled: &AtomicBool,
    left: &AtomicUsize,
) -> Result<Placed, Failure> {
    let root = Folder::root();
    let budget = Budget {
        left,
        taken: AtomicUsize::new(0),
    };
    let listing = Listing::of(fs, dir, true);
    let recorded = record(fs, listing, name, &root, cancelled, Some(&budget), None)
        .and_then(|()| root.take().pop().ok_or(Failure::Changed));
    match recorded {
        Ok((name, node)) => Ok(Placed { name, node }),
        Err(failure) => {
            left.fetch_add(budget.taken.into_inner(), Ordering::AcqRel);
            Err(failure)
        }
    }
}

pub(super) fn placed_proven(
    fs: &dyn FsOps,
    (dir, name): (BorrowedFd<'_>, &OsStr),
    cancelled: &AtomicBool,
    left: &AtomicUsize,
    ownership: &Ownership,
) -> Result<Placed, Failure> {
    let root = Folder::root();
    let budget = Budget {
        left,
        taken: AtomicUsize::new(0),
    };
    let recorded = record(
        fs,
        Listing::of(fs, dir, true),
        name,
        &root,
        cancelled,
        Some(&budget),
        Some((ownership, Path::new(""))),
    )
    .and_then(|()| root.take().pop().ok_or(Failure::Changed));
    match recorded {
        Ok((name, node)) => Ok(Placed { name, node }),
        Err(failure) => {
            left.fetch_add(budget.taken.into_inner(), Ordering::AcqRel);
            Err(failure)
        }
    }
}

struct Budget<'a> {
    left: &'a AtomicUsize,
    taken: AtomicUsize,
}

pub(super) fn of(parent: Arc<OwnedFd>, root: Arc<Folder>) -> Manifest {
    Manifest {
        parent: OffMain::new(parent),
        root,
        copies: Vec::new(),
    }
}

#[derive(Clone, Copy)]
struct Listing<'a> {
    dir: BorrowedFd<'a>,
    forced: bool,
}

impl<'a> Listing<'a> {
    fn of(fs: &dyn FsOps, dir: BorrowedFd<'a>, placed: bool) -> Self {
        let forced = placed || Mount::new(fs, dir).caches_attributes();
        Self { dir, forced }
    }
}

/// With `placed`, a file within a tick of its mount's coarse times is digested, for a deletion
/// to compare.
fn record(
    fs: &dyn FsOps,
    listing: Listing<'_>,
    name: &OsStr,
    folder: &Folder,
    cancelled: &AtomicBool,
    placed: Option<&Budget<'_>>,
    ownership: Option<(&Ownership, &Path)>,
) -> Result<(), Failure> {
    if cancelled.load(Ordering::Acquire) {
        return Err(Failure::Cancelled);
    }
    if let Some(budget) = placed {
        if budget
            .left
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(1)
            })
            .is_err()
        {
            return Err(Failure::Error(translated_io_error(
                "Too many entries to record",
            )));
        }
        budget.taken.fetch_add(1, Ordering::Relaxed);
    }
    let at = Entry {
        dir: listing.dir,
        name,
    };
    let stat = fs.stat(at, listing.forced)?;
    if ownership.is_some_and(|(owned, path)| !owned.unchanged(path, &stat)) {
        return Err(Failure::Changed);
    }
    if placed.is_some() && stat.kind == FileType::RegularFile {
        folder.add(name, digested(fs, at, &stat, cancelled)?);
        return Ok(());
    }
    if !stat.is_directory() {
        folder.entry(name, &stat);
        return Ok(());
    }
    let opened = fs.open_dir(at)?;
    if fs.stat_fd(opened.as_fd())?.identity != stat.identity {
        return Err(Failure::Changed);
    }
    let recorded = folder.folder(name, stat.identity);
    let children = Listing::of(fs, opened.as_fd(), placed.is_some());
    for (child, _) in fs.list(opened.as_fd())? {
        let child_path = ownership.map(|(_, path)| path.join(&child));
        record(
            fs,
            children,
            &child,
            &recorded,
            cancelled,
            placed,
            ownership
                .zip(child_path.as_deref())
                .map(|((owned, _), path)| (owned, path)),
        )
        .map_err(|failure| failure.named(&child))?;
    }
    Ok(())
}

fn digested(
    fs: &dyn FsOps,
    at: Entry<'_>,
    stat: &EntryStat,
    cancelled: &AtomicBool,
) -> Result<Node, Failure> {
    let Some(unsettled) = unsettled(stat, &Mount::new(fs, at.dir), now()) else {
        return Ok(Node::Entry(SourceSnapshot::of(stat, None)));
    };
    // Once the tick passes, any rewrite changes the times, so the bytes not
    // read by then need no digest.
    let stopped = || cancelled.load(Ordering::Acquire) || now() >= unsettled.settled;
    let read = || -> Result<Option<Option<Contents>>, Errno> {
        let Some((file, _)) = fs.open_source(at, false)? else {
            return Ok(None);
        };
        let end = stat.size.saturating_add(1);
        let (digest, prefix) = digest_of(fs, file.as_fd(), end, &stopped)?;
        let after = fs.stat_fd(file.as_fd())?;
        let contents = (prefix != Some(0)).then_some(Contents {
            digest,
            unsettled,
            prefix,
        });
        Ok((after.identity == stat.identity && !changed(stat, &after)).then_some(contents))
    };
    let read = read();
    if cancelled.load(Ordering::Acquire) {
        return Err(Failure::Cancelled);
    }
    Ok(match read {
        Ok(Some(contents)) => Node::Entry(SourceSnapshot::of(stat, contents)),
        // Nothing tells what was placed, so a deletion keeps it.
        _ => Node::Kept(KeptCause::Changed),
    })
}

fn digest_of(
    fs: &dyn FsOps,
    file: BorrowedFd<'_>,
    end: u64,
    stopped: &dyn Fn() -> bool,
) -> Result<(digest::Digest, Option<u64>), Errno> {
    let mut buffer = vec![0; usize::try_from(end).map_or(MIB, |end| end.clamp(64 << 10, MIB))];
    let mut digest = digest::Context::new(&digest::SHA256);
    let mut offset = 0;
    while offset < end {
        if stopped() {
            return Ok((digest.finish(), Some(offset)));
        }
        let wanted =
            usize::try_from(end - offset).map_or(buffer.len(), |left| left.min(buffer.len()));
        match fs.read_at(file, &mut buffer[..wanted], offset) {
            Ok(0) => break,
            Ok(read) => {
                digest.update(&buffer[..read]);
                offset += read as u64;
            }
            Err(Errno::INTR) => {}
            Err(error) => return Err(error),
        }
    }
    Ok((digest.finish(), None))
}

pub(super) fn delete(
    fs: &'static dyn FsOps,
    manifest: Manifest,
    unlinks: Arc<Unlinks>,
    cancelled: Arc<AtomicBool>,
) -> (Vec<(PathBuf, KeptCause)>, Result<(), glib::Error>) {
    let Manifest {
        parent,
        root,
        copies,
    } = manifest;
    // Deleting what only a copy that is gone held would lose it.
    if copies.iter().any(|copy| !copy.stays(fs)) {
        let kept = root
            .take()
            .into_iter()
            .map(|(name, _)| (PathBuf::from(name), KeptCause::CopyGone))
            .collect();
        return (kept, Ok(()));
    }
    let removed = remove(fs, parent.into_inner(), &root, unlinks, cancelled);
    (removed.kept, removed.result)
}

pub(in crate::adapters::local_operations) struct Removed {
    pub kept: Vec<(PathBuf, KeptCause)>,
    pub removed: bool,
    pub result: Result<(), glib::Error>,
}

pub(super) fn remove(
    fs: &'static dyn FsOps,
    parent: Arc<OwnedFd>,
    root: &Folder,
    unlinks: Arc<Unlinks>,
    cancelled: Arc<AtomicBool>,
) -> Removed {
    walk(fs, parent, root, unlinks, cancelled, false, false)
}

pub(super) fn remove_placed(
    fs: &'static dyn FsOps,
    parent: Arc<OwnedFd>,
    root: &Folder,
    unlinks: Arc<Unlinks>,
    cancelled: Arc<AtomicBool>,
) -> Removed {
    walk(fs, parent, root, unlinks, cancelled, false, true)
}

/// Keeps what [`remove`] would keep, and folders holding entries added
/// since, without deleting anything.
pub(super) fn compare(
    fs: &'static dyn FsOps,
    parent: Arc<OwnedFd>,
    root: &Folder,
    unlinks: Arc<Unlinks>,
    cancelled: Arc<AtomicBool>,
) -> Removed {
    walk(fs, parent, root, unlinks, cancelled, true, false)
}

fn walk(
    fs: &'static dyn FsOps,
    parent: Arc<OwnedFd>,
    root: &Folder,
    unlinks: Arc<Unlinks>,
    cancelled: Arc<AtomicBool>,
    compare_only: bool,
    own_folders: bool,
) -> Removed {
    let mount = Mount::new(fs, parent.as_fd());
    let identity = mount.identity_now(fs, parent.as_fd());
    let mut linked = HashMap::new();
    if counts_links(identity) && !compare_only {
        linked_names(root, Path::new(""), &mut linked);
    }
    let deletion = Arc::new(Deletion {
        fs,
        identity,
        root: parent.clone(),
        linked: Mutex::new(linked),
        unlinks,
        kept: Mutex::default(),
        removed: AtomicBool::new(false),
        compare_only,
        own_folders: own_folders.then_some(mount.family),
    });
    let steps = deletion.steps(root, Path::new(""));
    let workers = match steps.as_slice() {
        // A moved file has no folder to share out.
        [(_, _, Step::Entry(_))] => 1,
        _ => {
            let available = thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1);
            let rotational = fs.rotational(parent.as_fd()) == Some(true);
            bounded_local_worker_count(available, rotational)
        }
    };
    let queue = Arc::new(WorkQueue::new(cancelled));
    for (name, path, step) in steps {
        let at = At {
            deletion: deletion.clone(),
            parent: parent.clone(),
            name,
            path,
        };
        queue.enqueue(Removal {
            at,
            step,
            completion: None,
        });
    }
    work_queue::run_workers(&queue, workers, Some(10), |work| {
        thread::Builder::new().spawn(work)
    });
    Removed {
        kept: std::mem::take(&mut *lock(&deletion.kept)),
        removed: deletion.removed.load(Ordering::Acquire),
        result: queue.result(),
    }
}

fn linked_names(folder: &Folder, path: &Path, names: &mut HashMap<(u64, u64), Vec<PathBuf>>) {
    for (name, node) in lock(&folder.children).iter() {
        match node {
            Node::Entry(snapshot) if snapshot.nlink > 1 => names
                .entry((snapshot.identity.device, snapshot.identity.inode))
                .or_default()
                .push(path.join(name)),
            Node::Folder(folder) => linked_names(folder, &path.join(name), names),
            Node::Entry(_) | Node::Kept(_) => {}
        }
    }
}

struct Deletion {
    fs: &'static dyn FsOps,
    identity: IdentityTraits,
    root: Arc<OwnedFd>,
    linked: Mutex<HashMap<(u64, u64), Vec<PathBuf>>>,
    unlinks: Arc<Unlinks>,
    kept: Mutex<Vec<(PathBuf, KeptCause)>>,
    removed: AtomicBool,
    compare_only: bool,
    own_folders: Option<Family>,
}

impl Deletion {
    fn keep(&self, path: PathBuf, cause: KeptCause) {
        lock(&self.kept).push((path, cause));
    }

    fn holds_others(&self, at: Entry<'_>, path: &Path) -> bool {
        let kept: HashSet<OsString> = lock(&self.kept)
            .iter()
            .filter_map(|(kept, _)| kept.strip_prefix(path).ok()?.iter().next())
            .map(OsStr::to_os_string)
            .collect();
        self.fs
            .open_dir(at)
            .and_then(|opened| self.fs.list(opened.as_fd()))
            .map_or(true, |entries| {
                entries.iter().any(|(name, _)| !kept.contains(name))
            })
    }

    fn added_to(&self, opened: BorrowedFd<'_>, folder: &Folder) -> Result<bool, Errno> {
        let recorded: HashSet<OsString> = lock(&folder.children)
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        Ok(self
            .fs
            .list(opened)?
            .iter()
            .any(|(name, _)| !recorded.contains(name)))
    }

    fn failed(&self, path: PathBuf, error: Errno) {
        match error {
            // The copy holds it, and its source is gone, as a move leaves it.
            Errno::NOENT => {
                tracing::debug!(?path, "a moved source was gone before it was deleted");
            }
            error => self.keep(path, KeptCause::Undeletable(error.raw_os_error())),
        }
    }

    fn steps(&self, folder: &Folder, path: &Path) -> Vec<(OsString, PathBuf, Step)> {
        let mut steps = Vec::new();
        for (name, node) in folder.take() {
            let path = path.join(&name);
            let step = match node {
                Node::Kept(cause) => {
                    self.keep(path, cause);
                    continue;
                }
                Node::Entry(snapshot) => Step::Entry(snapshot),
                Node::Folder(folder) => Step::Folder(folder),
            };
            steps.push((name, path, step));
        }
        steps
    }

    fn same(&self, current: &EntryStat, identity: Option<LocalFileIdentity>) -> bool {
        identity.is_some_and(|identity| {
            native_fs::compare(self.identity, identity, None, current) != Proof::Different
        })
    }

    fn left_after_removing(
        &self,
        (key, path): ((u64, u64), &Path),
        identity: LocalFileIdentity,
    ) -> Option<(i64, u32)> {
        let mut linked = lock(&self.linked);
        let names = linked.get_mut(&key)?;
        names.retain(|name| name != path);
        names.iter().find_map(|name| {
            let other = Entry {
                dir: self.root.as_fd(),
                name: name.as_os_str(),
            };
            let stat = self.fs.stat(other, true).ok()?;
            self.same(&stat, Some(identity)).then_some(stat.ctime)
        })
    }

    fn loosen(&self, at: Entry<'_>, fd: &Arc<OwnedFd>, mode: u32) -> Option<Loosened> {
        let family = self.own_folders.filter(|_| !self.compare_only)?;
        (mode & 0o700 != 0o700 && restrictive::open_to_owner(self.fs, at, fd.as_fd(), mode, family))
            .then(|| Loosened {
                fs: self.fs,
                fd: fd.clone(),
                mode: Some(mode),
            })
    }
}

struct Loosened {
    fs: &'static dyn FsOps,
    fd: Arc<OwnedFd>,
    mode: Option<u32>,
}

impl Loosened {
    fn into_mode(mut self) -> Option<u32> {
        self.mode.take()
    }
}

impl Drop for Loosened {
    fn drop(&mut self) {
        if let Some(mode) = self.mode {
            let _ = self.fs.chmod_fd(self.fd.as_fd(), mode);
        }
    }
}

struct At {
    deletion: Arc<Deletion>,
    parent: Arc<OwnedFd>,
    name: OsString,
    path: PathBuf,
}

struct Removal {
    at: At,
    step: Step,
    completion: Option<Arc<Group<Removal>>>,
}

enum Step {
    Folder(Arc<Folder>),
    Entry(SourceSnapshot),
    Rmdir(Arc<OwnedFd>, Option<Loosened>),
}

impl Job for Removal {
    type Error = glib::Error;

    fn run(self, queue: &WorkQueue<Self>) {
        if queue.is_stopped() {
            return;
        }
        let Self {
            at,
            step,
            completion,
        } = self;
        match step {
            Step::Folder(folder) => at.folder(&folder, completion, queue),
            Step::Entry(snapshot) => {
                at.entry(&snapshot, &|| queue.is_stopped());
                queue.complete(completion);
            }
            Step::Rmdir(opened, loosened) => {
                at.rmdir(opened, loosened, &|| queue.is_stopped());
                queue.complete(completion);
            }
        }
    }

    fn cancelled() -> glib::Error {
        cancelled_local_operation()
    }

    fn panicked() -> glib::Error {
        translated_io_error("The deletion stopped unexpectedly")
    }

    fn not_started(error: io::Error) -> glib::Error {
        io_error(rust_i18n::t!(
            "Could not start the deletion: %{error}",
            error = error
        ))
    }
}

impl At {
    fn entry_at(&self) -> Entry<'_> {
        Entry::new(&*self.parent, &self.name)
    }

    fn folder(
        self,
        folder: &Folder,
        completion: Option<Arc<Group<Removal>>>,
        queue: &WorkQueue<Removal>,
    ) {
        let deletion = self.deletion.clone();
        let fs = deletion.fs;
        let opened = fs
            .open_dir(self.entry_at())
            .and_then(|fd| fs.stat_fd(fd.as_fd()).map(|stat| (fd, stat)))
            .and_then(|(fd, stat)| {
                let added = deletion.compare_only && deletion.added_to(fd.as_fd(), folder)?;
                Ok((fd, stat, added))
            });
        let (fd, mode) = match opened {
            Ok((fd, current, false)) if deletion.same(&current, folder.identity) => {
                (Arc::new(fd), current.mode)
            }
            Ok(_) | Err(Errno::NOTDIR | Errno::LOOP) => {
                deletion.keep(self.path, KeptCause::Changed);
                queue.complete(completion);
                return;
            }
            Err(error) => {
                deletion.failed(self.path, error);
                queue.complete(completion);
                return;
            }
        };
        let loosened = deletion.loosen(self.entry_at(), &fd, mode);
        let steps = deletion.steps(folder, &self.path);
        let rmdir = Removal {
            at: self,
            step: Step::Rmdir(fd.clone(), loosened),
            completion,
        };
        if steps.is_empty() {
            rmdir.run(queue);
            return;
        }
        let group = Group::new(steps.len(), rmdir);
        for (name, path, step) in steps {
            let at = Self {
                deletion: deletion.clone(),
                parent: fd.clone(),
                name,
                path,
            };
            queue.enqueue(Removal {
                at,
                step,
                completion: Some(group.clone()),
            });
        }
    }

    fn entry(&self, snapshot: &SourceSnapshot, stopped: &dyn Fn() -> bool) {
        let deletion = &self.deletion;
        let fs = deletion.fs;
        let ahead = snapshot
            .contents
            .as_ref()
            .and_then(|contents| contents.unsettled.ahead)
            .filter(|&modified| now() < modified);
        // Compared before the stat below, which sees any later rewrite.
        let compared = match &snapshot.contents {
            Some(contents) if ahead.is_none() => {
                match self.same_contents(contents, snapshot.size, stopped) {
                    None => return,
                    Some(Ok(Some(file))) => Some(file),
                    Some(Ok(None)) => return deletion.keep(self.path.clone(), KeptCause::Changed),
                    Some(Err(error)) => return deletion.failed(self.path.clone(), error),
                }
            }
            _ => None,
        };
        let key = (snapshot.identity.device, snapshot.identity.inode);
        // Checking, removing and counting a name of a file with several
        // names must not interleave with another name's check.
        let mut unlinks = (snapshot.nlink > 1).then(|| lock(&deletion.unlinks.unlinked));
        let since = unlinks
            .as_ref()
            .and_then(|unlinks| unlinks.get(&key))
            .map_or((0, None), |unlinked| unlinked.since(snapshot.nlink));
        let current = fs.stat(self.entry_at(), true);
        // Only the file compared may go, not one renamed over its name since;
        // its open descriptor keeps its inode number. Closed before the
        // unlink, as FUSE and NFS rename a file unlinked while open aside.
        let compared_here = compared.is_none_or(|file| {
            let opened = fs.stat_fd(file.as_fd());
            matches!((&current, opened), (Ok(current), Ok(opened)) if opened.identity == current.identity)
        });
        // Left uncompared only while the clock has not reached its time.
        let still_ahead = ahead.is_none_or(|modified| now() < modified);
        let unchanged = compared_here
            && still_ahead
            && current
                .as_ref()
                .is_ok_and(|current| snapshot.holds(current, deletion.identity, since));
        match current {
            Ok(_) if unchanged && deletion.compare_only => {}
            Ok(current) if unchanged => match fs.unlink(self.entry_at(), AtFlags::empty()) {
                Ok(()) => {
                    deletion.removed.store(true, Ordering::Release);
                    if let Some(unlinks) = &mut unlinks {
                        let left =
                            deletion.left_after_removing((key, &self.path), snapshot.identity);
                        let unlinked = unlinks.entry(key).or_default();
                        unlinked.counts.push(current.nlink);
                        unlinked.left = left;
                    }
                }
                Err(error) => deletion.failed(self.path.clone(), error),
            },
            Ok(_) => deletion.keep(self.path.clone(), KeptCause::Changed),
            Err(error) => deletion.failed(self.path.clone(), error),
        }
    }

    /// The file, still open, if it holds what the copy holds; `None` if the
    /// deletion stopped meanwhile.
    fn same_contents(
        &self,
        contents: &Contents,
        size: u64,
        stopped: &dyn Fn() -> bool,
    ) -> Option<Result<Option<OwnedFd>, Errno>> {
        match wait_until(contents.unsettled, stopped) {
            Waited::Settled => {}
            Waited::Stopped => return None,
            Waited::SetBack => return Some(Ok(None)),
        }
        let fs = self.deletion.fs;
        let file = match fs.open_source(self.entry_at(), false) {
            Ok(Some((file, _))) => file,
            Ok(None) => return Some(Ok(None)),
            Err(error) => return Some(Err(error)),
        };
        // A byte past what the copy holds tells a file that grew.
        let end = contents.prefix.unwrap_or(size.saturating_add(1));
        match digest_of(fs, file.as_fd(), end, stopped) {
            Ok((_, Some(_))) => None,
            Ok((digest, None)) => Some(Ok(
                (digest.as_ref() == contents.digest.as_ref()).then_some(file)
            )),
            Err(error) => Some(Err(error)),
        }
    }

    fn rmdir(&self, opened: Arc<OwnedFd>, loosened: Option<Loosened>, stopped: &dyn Fn() -> bool) {
        let deletion = &self.deletion;
        if deletion.compare_only {
            return;
        }
        let fs = deletion.fs;
        // An open folder keeps its inode number even where numbers change
        // between the copy and the deletion, so the name must report it.
        let named = fs
            .stat(self.entry_at(), false)
            .and_then(|named| Ok((named, fs.stat_fd(opened.as_fd())?)));
        let identity = match named {
            Ok((named, opened)) if named.identity == opened.identity => opened.identity,
            Ok(_) => return deletion.keep(self.path.clone(), KeptCause::Changed),
            Err(error) => return deletion.failed(self.path.clone(), error),
        };
        let mode = loosened.and_then(Loosened::into_mode);
        drop(opened);
        let leftovers = &deletion.unlinks.leftovers;
        let unchanged = || {
            fs.stat(self.entry_at(), false)
                .is_ok_and(|current| deletion.same(&current, Some(identity)))
        };
        match leftovers::remove_folder(fs, self.entry_at(), leftovers, stopped, &unchanged) {
            Ok(()) => return deletion.removed.store(true, Ordering::Release),
            Err(Refused::Os(Errno::NOENT)) => return,
            Err(Refused::Changed(_)) => {
                return deletion.keep(self.path.clone(), KeptCause::Changed);
            }
            Err(Refused::Stopped(_)) => {}
            Err(Refused::StillOpen(errno)) => match leftovers::holder(fs, self.entry_at()) {
                None if errno == Errno::ACCESS => deletion.failed(self.path.clone(), errno),
                holder => deletion.keep(self.path.clone(), KeptCause::StillOpen(holder)),
            },
            // Entries added meanwhile stay, and so does their folder.
            Err(Refused::Os(Errno::NOTEMPTY | Errno::EXIST)) => {
                if deletion.holds_others(self.entry_at(), &self.path) {
                    deletion.keep(self.path.clone(), KeptCause::Changed);
                }
            }
            Err(refused) => deletion.failed(self.path.clone(), refused.errno()),
        }
        if let Some(mode) = mode
            && let Ok(folder) = fs.open_dir(self.entry_at())
            && fs
                .stat_fd(folder.as_fd())
                .is_ok_and(|current| deletion.same(&current, Some(identity)))
        {
            let _ = fs.chmod_fd(folder.as_fd(), mode);
        }
    }
}
