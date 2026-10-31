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

use gtk::glib;
use rustix::{
    fs::{AtFlags, FileType, Mode},
    io::Errno,
};

use super::{
    Barrier, Copy, Created, Failure, Mount, Placing, Source, best_effort, creation_failure,
    data::Data,
    direct_folder, from_stage, is_fatal, is_unsupported,
    links::{self, Links, Role},
    lock,
    manifest::{self, Pending},
    metadata::Snapshot,
    open_failure, open_file, open_one_name,
    restrictive::{self, Restrictive},
    stage,
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

    fn reach(
        &self,
        fs: &dyn FsOps,
        mount: &Mount,
        dir: BorrowedFd<'_>,
        path: &Path,
    ) -> Result<(), Failure> {
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
        fixing: bool,
        key: Key,
    ) -> Result<Vec<(OsString, FileType, Planned)>, Failure> {
        target_children(self.copy.fs, source, &rules.fixed, fixing, key)
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Naming {
    Decide,
    Create,
    /// The filesystem may take two names for one, so names that may collide
    /// are claimed in byte order before their files are written.
    Claim,
}

impl Naming {
    fn key(self) -> Key {
        if self == Self::Create {
            Key::Exact
        } else {
            Key::Folded
        }
    }

    fn decide(self, fs: &dyn FsOps, family: Family, folder: BorrowedFd<'_>) -> Self {
        if self != Self::Decide {
            return self;
        }
        let never_collide = stage::names_never_collide(family)
            && fs
                .file_flags(folder)
                .is_ok_and(|flags| flags & CASEFOLD == 0);
        if never_collide {
            Self::Create
        } else {
            Self::Claim
        }
    }
}

/// The names of a folder this copy made, against which its aliases are
/// numbered. A numbered name never takes a name an entry of the folder asks
/// for, and each is tried by one job only, so a name found taken is never
/// another job's entry mistaken for one whose reply was lost.
pub(super) struct Names {
    requested: Vec<OsString>,
    keys: OnceLock<HashMap<OsString, usize>>,
    numbered: Mutex<HashSet<OsString>>,
    /// What the folder's jobs made, by collision key, where a taken name may
    /// be the copy's own entry whose reply was lost; `None` elsewhere. Some
    /// mounts list a name in another case than it was made in, so the
    /// listing alone cannot tell such an entry from another name's.
    made: Option<Mutex<HashMap<OsString, Vec<LocalFileIdentity>>>>,
    identity: IdentityTraits,
    rules: FolderRules,
}

impl Names {
    fn new(
        requested: Vec<OsString>,
        naming: Naming,
        identity: IdentityTraits,
        rules: FolderRules,
    ) -> Arc<Self> {
        Arc::new(Self {
            requested,
            keys: OnceLock::new(),
            numbered: Mutex::default(),
            made: (naming == Naming::Claim).then(Mutex::default),
            identity,
            rules,
        })
    }

    fn adopts(&self) -> bool {
        self.made.is_some()
    }

    pub(super) fn made(&self, name: &OsStr, identity: LocalFileIdentity) {
        if let Some(made) = &self.made {
            lock(made)
                .entry(stage::collision_key(name))
                .or_default()
                .push(identity);
        }
    }

    fn unclaimed(&self, name: &OsStr, current: &EntryStat) -> bool {
        let Some(made) = &self.made else {
            return false;
        };
        lock(made)
            .get(&stage::collision_key(name))
            .is_none_or(|made| {
                made.iter()
                    .all(|&made| compare(self.identity, made, None, current) == Proof::Different)
            })
    }

    fn requested(&self) -> &HashMap<OsString, usize> {
        self.keys.get_or_init(|| {
            let mut keys = HashMap::new();
            for name in &self.requested {
                *keys.entry(stage::collision_key(name)).or_default() += 1;
            }
            keys
        })
    }

    fn claims(&self, name: &OsStr, own: Option<&OsStr>) -> bool {
        let key = stage::collision_key(name);
        let asked = match self.requested().get(&key) {
            None => false,
            Some(1) => own.is_none_or(|own| stage::collision_key(own) != key),
            Some(_) => true,
        };
        !asked && lock(&self.numbered).insert(key)
    }

    fn number<T>(
        &self,
        name: &OsStr,
        is_directory: bool,
        mut make: impl FnMut(&OsStr) -> Result<T, Failure>,
    ) -> Result<(OsString, T), Failure> {
        for candidate in numbered_names(name, is_directory, self.rules.limit) {
            if !self.claims(&candidate, None) {
                continue;
            }
            match make(&candidate) {
                Err(Failure::Place(PlaceError::Exists)) => {}
                made => return made.map(|value| (candidate, value)),
            }
        }
        Err(Failure::Place(PlaceError::Exists))
    }

    fn stricter<T>(
        &self,
        (refused, name): (&OsStr, &OsStr),
        is_directory: bool,
        mut make: impl FnMut(&OsStr) -> Result<T, Failure>,
    ) -> Result<(OsString, T), Failure> {
        if self.claims(name, Some(refused)) {
            match make(name) {
                Err(Failure::Place(PlaceError::Exists)) => {}
                made => return made.map(|value| (name.to_os_string(), value)),
            }
        }
        self.number(name, is_directory, make)
    }
}

pub(super) fn make_numbered<T>(
    copy: &Copy,
    names: &Names,
    path: &Path,
    is_directory: bool,
    cause: RenameCause,
    mut make: impl FnMut(&OsStr) -> Result<T, Failure>,
) -> Result<(PathBuf, T), Failure> {
    let name = path.file_name().unwrap_or_default();
    let (saved, made, cause) = match make(name) {
        Err(Failure::Place(PlaceError::Exists)) => {
            let (saved, made) = names.number(name, is_directory, make)?;
            (saved, made, cause)
        }
        Err(refused) if refused.refuses_name() => {
            let target = (name, is_directory);
            super::retry_refused(copy, &names.rules, target, refused, |stricter| {
                names.stricter((name, stricter), is_directory, &mut make)
            })?
        }
        made => return made.map(|value| (path.to_path_buf(), value)),
    };
    let saved = path.with_file_name(saved);
    copy.renamed(path.to_path_buf(), saved.clone(), cause);
    Ok((saved, made))
}

fn planned_rename(copy: &Copy, folder: &Path, name: &OsStr, planned: &Planned) {
    if let Some(cause) = planned.cause {
        copy.renamed(folder.join(name), folder.join(&planned.target), cause);
    }
}

pub(super) struct Folder {
    pub(super) source: (Arc<OwnedFd>, OsString, Option<LocalFileIdentity>),
    pub(super) target: (Arc<OwnedFd>, OsString, LocalFileIdentity),
    pub(super) parent: EntryStat,
    pub(super) path: PathBuf,
    pub(super) naming: Naming,
    pub(super) site: Site,
    pub(super) mount: Mount,
    /// Names link only within one scope, whose restrictive folders get their
    /// modes together: the made tree, or one merge stage.
    pub(super) scope: usize,
    pub(super) recorded_in: Option<Arc<manifest::Folder>>,
    pub(super) completion: Completion,
}

impl Folder {
    fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        let name = self.source.1.clone();
        if let Err(failure) = self.fill(tree, queue) {
            queue.fail(failure.into_error(&name));
        }
    }

    fn fill(self, tree: &Arc<Tree>, queue: &Queue) -> Result<(), Failure> {
        let copy = &tree.copy;
        let fs = copy.fs;
        let (source_parent, source_name, expected) = &self.source;
        let source = fs
            .open_dir(Entry::new(&**source_parent, source_name))
            .map_err(|error| open_failure(source_name, error))?;
        let source_stat = fs.stat_fd(source.as_fd())?;
        let (source_mount, _) = tree.source_in(fs, source.as_fd());
        if expected.is_some_and(|expected| !source_mount.may_be(expected, &source_stat)) {
            return Err(Failure::Changed);
        }
        let (target_parent, target_name, created) = &self.target;
        let target = fs.open_dir(Entry::new(&**target_parent, target_name))?;
        let target_stat = fs.stat_fd(target.as_fd())?;
        if !self.mount.may_be(*created, &target_stat) {
            return Err(Failure::Place(PlaceError::Changed));
        }
        let snapshot = Snapshot::read(fs, source.as_fd(), &source_stat)?;
        let naming = self.naming.decide(fs, self.mount.family, target.as_fd());
        let fixing = tree.crosses(&source_stat, &target_stat, &self.site);
        let rules = copy.names.in_folder(&self.site);
        let children = tree.plan(source.as_fd(), &rules, fixing, naming.key())?;
        let requested = children
            .iter()
            .map(|(_, _, planned)| planned.target.clone())
            .collect();
        let made = Made {
            naming,
            dir: Arc::new(target),
            stat: target_stat,
            path: self.path,
            names: Names::new(requested, naming, self.mount.identity, rules),
            site: self.site,
            mount: self.mount,
            scope: self.scope,
            recorded: self
                .recorded_in
                .map(|folder| folder.folder(source_name, source_stat.identity)),
        };
        let finish = Finish {
            name: source_name.clone(),
            target: made.dir.clone(),
            created: target_stat,
            parent: self.parent,
            snapshot,
            path: made.path.clone(),
            mount: made.mount,
            scope: self.scope,
            completion: self.completion,
        };
        if children.is_empty() {
            finish.run(tree, queue);
            return Ok(());
        }
        let group = Group::new(children.len(), TreeJob::Finish(tree.clone(), finish));
        made.make_all(tree, queue, &Arc::new(source), children, &group)
    }
}

fn target_children(
    fs: &dyn FsOps,
    source: BorrowedFd<'_>,
    rules: &NameProfile,
    fixing: bool,
    key: Key,
) -> Result<Vec<(OsString, FileType, Planned)>, Failure> {
    let children = children(fs, source)?;
    let entries = children
        .iter()
        .map(|(name, kind)| PlanEntry {
            name,
            is_directory: *kind == FileType::Directory,
            fixing,
            repeatable: false,
        })
        .collect::<Vec<_>>();
    let planned = plan_names(&entries, rules, key)?;
    Ok(children
        .into_iter()
        .zip(planned)
        .map(|((name, kind), planned)| (name, kind, planned))
        .collect())
}

struct Made {
    dir: Arc<OwnedFd>,
    stat: EntryStat,
    path: PathBuf,
    naming: Naming,
    names: Arc<Names>,
    site: Site,
    mount: Mount,
    scope: usize,
    recorded: Option<Arc<manifest::Folder>>,
}

struct Child<'a> {
    name: &'a OsStr,
    kind: FileType,
    target_name: &'a OsStr,
    claim: bool,
}

impl Made {
    fn make_all(
        &self,
        tree: &Arc<Tree>,
        queue: &Queue,
        source: &Arc<OwnedFd>,
        children: Vec<(OsString, FileType, Planned)>,
        group: &Arc<Group<TreeJob>>,
    ) -> Result<(), Failure> {
        let colliding = if self.naming == Naming::Claim {
            let names = children
                .iter()
                .map(|(_, _, planned)| planned.target.as_os_str())
                .collect::<Vec<_>>();
            stage::colliding(&names)
        } else {
            vec![false; children.len()]
        };
        for ((name, kind, planned), colliding) in children.into_iter().zip(colliding) {
            tree.copy.check_cancelled()?;
            let child = Child {
                name: &name,
                kind,
                target_name: &planned.target,
                claim: colliding,
            };
            let made = self
                .make(tree, queue, source, &child, group)
                .map_err(|failure| failure.named(&name))?;
            if made {
                planned_rename(&tree.copy, &self.path, &name, &planned);
            }
        }
        Ok(())
    }

    fn make(
        &self,
        tree: &Arc<Tree>,
        queue: &Queue,
        source: &Arc<OwnedFd>,
        child: &Child<'_>,
        group: &Arc<Group<TreeJob>>,
    ) -> Result<bool, Failure> {
        let copy = &tree.copy;
        let fs = copy.fs;
        let name = child.name;
        let path = self.path.join(child.target_name);
        let names = &*self.names;
        let alias = RenameCause::Alias;
        let dir = &*self.dir;
        match child.kind {
            FileType::Directory => {
                let (path, created) = make_numbered(copy, names, &path, true, alias, |name| {
                    claim_folder(fs, Entry::new(dir, name), names)
                })?;
                tree.made(&path, created);
                queue.enqueue(TreeJob::Folder(
                    tree.clone(),
                    Folder {
                        source: (source.clone(), name.to_os_string(), None),
                        target: (self.dir.clone(), file_name(&path), created),
                        parent: self.stat,
                        path,
                        naming: self.naming,
                        site: self.site,
                        mount: self.mount,
                        scope: self.scope,
                        recorded_in: self.recorded.clone(),
                        completion: Some(group.clone()),
                    },
                ));
            }
            FileType::RegularFile => {
                let (path, claimed) = if child.claim {
                    let (path, identity) =
                        make_numbered(copy, names, &path, false, alias, |name| {
                            claim_file(copy, Entry::new(dir, name), names, self.mount.family)
                        })?;
                    tree.made(&path, identity);
                    (path, Some(identity))
                } else {
                    (path, None)
                };
                queue.enqueue(TreeJob::File(
                    tree.clone(),
                    File {
                        source: source.clone(),
                        name: name.to_os_string(),
                        target: self.dir.clone(),
                        parent: self.stat,
                        claimed,
                        names: self.names.clone(),
                        mount: self.mount,
                        path,
                        scope: self.scope,
                        recorded_in: self.recorded.clone(),
                        completion: Some(group.clone()),
                    },
                ));
            }
            FileType::Symlink => {
                let source = Entry::new(&**source, name);
                let made = copy.names.profiles.holds_links(&self.site).then(|| {
                    make_numbered(copy, names, &path, false, alias, |name| {
                        copy_link(
                            copy,
                            source,
                            Entry::new(dir, name),
                            &self.mount,
                            Some((names, &self.site)),
                        )
                    })
                });
                let (path, link) = match made {
                    Some(Ok(made)) => made,
                    Some(Err(failure)) if copy.refused_link(&self.site, &failure).is_none() => {
                        return Err(failure);
                    }
                    _ => {
                        self.skip(copy, name, SpecialKind::Symlink)?;
                        queue.complete(Some(group.clone()));
                        return Ok(false);
                    }
                };
                if tree.created.is_some() || names.adopts() {
                    let name = file_name(&path);
                    let made = fs.stat(Entry::new(dir, &name), false)?;
                    tree.made(&path, made.identity);
                    names.made(&name, made.identity);
                    copy.record_state(made.identity, &made);
                }
                self.record(name, &link);
                queue.complete(Some(group.clone()));
            }
            FileType::Fifo => {
                let source = Entry::new(&**source, name);
                let (path, (pipe, made)) =
                    make_numbered(copy, names, &path, false, alias, |name| {
                        copy_fifo(
                            copy,
                            source,
                            Entry::new(dir, name),
                            &self.stat,
                            &self.mount,
                            Some(names),
                        )
                    })?;
                match made {
                    Some(identity) => {
                        tree.made(&path, identity);
                        names.made(&file_name(&path), identity);
                        self.record(name, &pipe);
                    }
                    None => {
                        self.skip(copy, name, SpecialKind::Pipe)?;
                        queue.complete(Some(group.clone()));
                        return Ok(false);
                    }
                }
                queue.complete(Some(group.clone()));
            }
            kind => {
                self.skip(copy, name, special_kind(kind))?;
                queue.complete(Some(group.clone()));
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Reported under its own name, since nothing took the planned one.
    fn skip(&self, copy: &Copy, name: &OsStr, kind: SpecialKind) -> Result<(), Failure> {
        copy.skip(self.path.join(name), (name, self.recorded.as_deref()), kind)
    }

    fn record(&self, name: &OsStr, stat: &EntryStat) {
        if let Some(folder) = &self.recorded {
            folder.entry(name, stat);
        }
    }
}

fn special_kind(kind: FileType) -> SpecialKind {
    if kind == FileType::Socket {
        SpecialKind::Socket
    } else {
        SpecialKind::Device
    }
}

fn file_name(path: &Path) -> OsString {
    path.file_name().unwrap_or_default().to_os_string()
}

fn children(fs: &dyn FsOps, folder: BorrowedFd<'_>) -> Result<Vec<(OsString, FileType)>, Failure> {
    let mut children = fs.list(folder)?;
    children.retain(|(name, kind)| {
        *kind == FileType::Directory || !leftovers::is_leftover(fs, folder, name)
    });
    children.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (name, kind) in &mut children {
        if *kind == FileType::Unknown {
            *kind = fs
                .stat(Entry { dir: folder, name }, false)
                .map_err(|error| Failure::Os(error).named(name))?
                .kind;
        }
    }
    Ok(children)
}

/// sshfs reports a taken name as `EPERM`, and SMB a folder in a file's way
/// as `EISDIR`.
pub(in crate::adapters::local_operations) fn taken(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    error: Errno,
) -> bool {
    match error {
        Errno::EXIST | Errno::ISDIR => true,
        Errno::PERM => fs.stat(entry, false).is_ok(),
        _ => false,
    }
}

/// NFS creations are not idempotent, so a taken name may be this copy's own
/// entry whose reply was lost.
pub(super) fn adopt(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    names: Option<&Names>,
    fresh: impl FnOnce(&EntryStat) -> bool,
) -> Result<LocalFileIdentity, Failure> {
    let ours = || {
        let names = names?;
        let stat = fs.stat(entry, true).ok()?;
        if !fresh(&stat) || !names.unclaimed(entry.name, &stat) {
            return None;
        }
        let listed = fs
            .names(Entry {
                dir: entry.dir,
                name: OsStr::new("."),
            })
            .ok()?
            .iter()
            .any(|name| name == entry.name);
        listed.then(|| {
            names.made(entry.name, stat.identity);
            stat.identity
        })
    };
    ours().ok_or(Failure::Place(PlaceError::Exists))
}

fn claim_folder(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    names: &Names,
) -> Result<LocalFileIdentity, Failure> {
    match fs.mkdir(entry, Mode::from_raw_mode(0o700)) {
        Ok(()) => {
            let identity = fs.stat(entry, false)?.identity;
            names.made(entry.name, identity);
            Ok(identity)
        }
        Err(error) if taken(fs, entry, error) => adopt(fs, entry, Some(names), |stat| {
            stat.is_directory() && fs.names(entry).is_ok_and(|names| names.is_empty())
        }),
        Err(error) => Err(creation_failure(fs, entry, error)),
    }
}

/// Claims a file's name without opening it for writing: vfat mounted with
/// `flush` waits at every such close, and the file job opens it once more.
/// On SMB, names made as nodes and looked up while other files were written
/// left the folder's rename waiting 35 s for Samba, so they are created.
fn claim_file(
    copy: &Copy,
    entry: Entry<'_>,
    names: &Names,
    family: Family,
) -> Result<LocalFileIdentity, Failure> {
    let fs = copy.fs;
    if family != Family::Smb && !copy.nodes_refused.load(Ordering::Relaxed) {
        match fs.create_node(entry, FileType::RegularFile, Mode::from_raw_mode(0o600)) {
            Ok(()) => {
                let identity = fs.stat(entry, false)?.identity;
                names.made(entry.name, identity);
                return Ok(identity);
            }
            Err(error) if taken(fs, entry, error) => {
                return adopt(fs, entry, Some(names), fresh_file);
            }
            Err(error)
                if error == Errno::PERM || error == Errno::NOSYS || is_unsupported(error) =>
            {
                copy.nodes_refused.store(true, Ordering::Relaxed);
            }
            Err(error) => return Err(creation_failure(fs, entry, error)),
        }
    }
    match fs.create_readonly(entry, Mode::from_raw_mode(0o600)) {
        Ok(fd) => {
            let identity = fs.stat_fd(fd.as_fd())?.identity;
            fs.close(fd)?;
            names.made(entry.name, identity);
            Ok(identity)
        }
        Err(error) if taken(fs, entry, error) => adopt(fs, entry, Some(names), fresh_file),
        Err(error) => Err(creation_failure(fs, entry, error)),
    }
}

fn fresh_file(stat: &EntryStat) -> bool {
    stat.kind == FileType::RegularFile && stat.size == 0 && stat.nlink == 1
}

/// A link the placement made again at its name is another entry, which holds
/// the copy only with the staged link's target.
pub(super) fn made_again(
    fs: &dyn FsOps,
    at: Entry<'_>,
    link: Option<&OsStr>,
) -> manifest::Ownership {
    match (fs.stat(at, false), link) {
        (Ok(made), Some(link))
            if made.kind == FileType::Symlink
                && fs.readlink(at).is_ok_and(|current| current == link) =>
        {
            manifest::Ownership::of(&made)
        }
        _ => manifest::Ownership::unproven(),
    }
}

pub(super) fn read_link(fs: &dyn FsOps, at: Entry<'_>) -> Result<OsString, Failure> {
    fs.readlink(at).map_err(|error| match error {
        Errno::INVAL => Failure::Changed,
        error => error.into(),
    })
}

/// Without `named`, the link is made from a stage, whose target was checked.
pub(super) fn copy_link(
    copy: &Copy,
    source: Entry<'_>,
    target: Entry<'_>,
    mount: &Mount,
    named: Option<(&Names, &Site)>,
) -> Result<EntryStat, Failure> {
    let fs = copy.fs;
    let stat = copy.source_stat(source)?;
    if stat.kind != FileType::Symlink {
        return Err(Failure::Changed);
    }
    let link = read_link(fs, source)?;
    let names = named.map(|(names, _)| names);
    if named.is_some_and(|(_, site)| !copy.names.profiles.holds_link_to(site, &link)) {
        return Err(Failure::LinkRefused(Errno::ILSEQ));
    }
    if let Err(error) = fs.symlink(&link, target) {
        if !taken(fs, target, error) {
            return Err(match creation_failure(fs, target, error) {
                Failure::Os(error) if refuses_node(FileType::Symlink, error) => {
                    Failure::LinkRefused(error)
                }
                failure => failure,
            });
        }
        adopt(fs, target, names, |stat| {
            stat.kind == FileType::Symlink
                && fs.readlink(target).is_ok_and(|current| current == link)
        })?;
    }
    if read_link(fs, target)? != link {
        return Err(Failure::Changed);
    }
    // sshfs sets a link's times on what it points to.
    if mount.family != Family::Fuse
        && let Err(error) = best_effort(fs.set_link_times(target, stat.atime, stat.mtime))
    {
        if names.is_some() && fs.readlink(target).is_ok_and(|current| current == link) {
            let _ = fs.unlink(target, AtFlags::empty());
        }
        return Err(error.into());
    }
    copy.add_file(stat.size);
    Ok(stat)
}

/// The source's xattrs are not copied: reading them would open it, which
/// lets a writer waiting on it go on.
pub(super) fn copy_fifo(
    copy: &Copy,
    source: Entry<'_>,
    target: Entry<'_>,
    parent: &EntryStat,
    mount: &Mount,
    names: Option<&Names>,
) -> Result<(EntryStat, Option<LocalFileIdentity>), Failure> {
    let fs = copy.fs;
    let stat = copy.source_stat(source)?;
    if stat.kind != FileType::Fifo {
        return Err(Failure::Changed);
    }
    let identity = match fs.create_node(target, FileType::Fifo, Mode::from_raw_mode(0o600)) {
        Ok(()) => fs.stat(target, false)?.identity,
        Err(error) if taken(fs, target, error) => {
            adopt(fs, target, names, |stat| stat.kind == FileType::Fifo)?
        }
        Err(error) if refuses_node(FileType::Fifo, error) => return Ok((stat, None)),
        Err(error) => return Err(creation_failure(fs, target, error)),
    };
    if identity.kind != FileType::Fifo {
        return Err(Failure::Changed);
    }
    let finished = finish_pipe(copy, &stat, (target, identity), parent, mount);
    // An unprovable public creation cannot authorize deleting a concurrent arrival.
    if finished.is_err() && names.is_some() {
        placement::remove_if_unchanged(fs, target, identity, None, AtFlags::empty());
    }
    finished?;
    copy.add_file(0);
    Ok((stat, Some(identity)))
}

pub(super) fn finish_pipe(
    copy: &Copy,
    stat: &EntryStat,
    (target, identity): (Entry<'_>, LocalFileIdentity),
    parent: &EntryStat,
    mount: &Mount,
) -> Result<(), Failure> {
    let fs = copy.fs;
    let fd = match fs.open_fifo(target) {
        Ok(fd) => fd,
        Err(error) if is_fatal(error) => return Err(error.into()),
        Err(_) => return Ok(()),
    };
    let created = fs.stat_fd(fd.as_fd())?;
    if created.kind != FileType::Fifo || !mount.may_be(identity, &created) {
        return Err(Failure::Place(PlaceError::Changed));
    }
    Snapshot::of(stat).apply_to_pipe(
        fs,
        fd.as_fd(),
        &created,
        parent,
        mount,
        &copy.refused_xattrs,
    )?;
    best_effort(fs.close(fd)).map_err(Failure::Os)
}

/// How filesystems refuse a node of `kind` they cannot hold. Without named
/// pipes, vfat answers `EPERM`, exfat-fuse `EIO` and SMB `EOPNOTSUPP`;
/// without symbolic links, vfat answers `EPERM`, exfat-fuse `ENOSYS` and
/// SMB `EOPNOTSUPP`, and ntfs3 refuses a target too long with `EFBIG`.
pub(in crate::adapters::local_operations) fn refuses_node(kind: FileType, error: Errno) -> bool {
    let refused = match kind {
        FileType::Fifo => &[Errno::PERM, Errno::IO, Errno::NOSYS, Errno::INVAL][..],
        FileType::Symlink => &[Errno::PERM, Errno::NOSYS, Errno::FBIG],
        _ => return false,
    };
    refused.contains(&error) || is_unsupported(error)
}

pub(super) struct File {
    source: Arc<OwnedFd>,
    name: OsString,
    target: Arc<OwnedFd>,
    parent: EntryStat,
    claimed: Option<LocalFileIdentity>,
    names: Arc<Names>,
    mount: Mount,
    path: PathBuf,
    scope: usize,
    recorded_in: Option<Arc<manifest::Folder>>,
    completion: Completion,
}

impl File {
    fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        #[cfg(test)]
        let _activity = crate::adapters::local_operations::tests::CopyActivity::start();
        match tree.copy.with_data(|data| self.copy(tree, data)) {
            Ok(role) => {
                if role != Role::Later {
                    tree.copy.add_file(0);
                }
                queue.complete(self.completion);
            }
            Err(failure) => queue.fail(failure.into_error(&self.name)),
        }
    }

    fn copy(&self, tree: &Tree, data: &mut Data) -> Result<Role, Failure> {
        let copy = &*tree.copy;
        let fs = copy.fs;
        copy.check_cancelled()?;
        if copy.counted {
            copy.progress.start_file(&self.name);
        }
        let at = Entry::new(&*self.source, &self.name);
        // An SMB client opens one name of a file at a time, so a later name
        // of a linked file is never opened there, and the others take turns.
        let (opened, stat) = if tree.opens_one_name_per_file() {
            let stat = copy
                .source_stat(at)
                .map_err(|error| open_failure(&self.name, error))?;
            (None, stat)
        } else {
            let opened = open_file(fs, at, &copy.cancelled)?;
            let stat = opened.stat;
            (Some(opened), stat)
        };
        let key = (self.scope, stat.identity.device, stat.identity.inode);
        let role = if stat.nlink > 1 {
            let name = links::Name {
                key,
                path: self.path.clone(),
                source: self.name.clone(),
                claimed: self.claimed,
                names: self.names.clone(),
                size: stat.size,
                recorded: self.recorded_in.clone().map(|folder| {
                    let openable = !tree.opens_one_name_per_file();
                    Pending::new(folder, &self.name, &stat, openable)
                }),
            };
            tree.links.role(fs, &self.mount, name)
        } else {
            Role::Independent
        };
        if role == Role::Later {
            return Ok(role);
        }
        let source = match opened {
            Some(opened) => opened,
            None => open_one_name(fs, at, &stat, &copy.cancelled)?,
        };
        let dir = &*self.target;
        let (path, (fd, expected)) = match self.claimed {
            Some(claimed) => {
                let fd = open_made(fs, Entry::new(dir, &file_name(&self.path)))?;
                (self.path.clone(), (fd, Some(claimed)))
            }
            None => make_numbered(
                copy,
                &self.names,
                &self.path,
                false,
                RenameCause::Alias,
                |name| {
                    let target = Entry::new(dir, name);
                    match fs.create_exclusive(target, Mode::from_raw_mode(0o600)) {
                        Ok(fd) => Ok((fd, None)),
                        Err(error) if !taken(fs, target, error) => {
                            Err(creation_failure(fs, target, error))
                        }
                        Err(_) if self.names.adopts() => {
                            let identity = adopt(fs, target, Some(&self.names), fresh_file)?;
                            tree.made(&self.path.with_file_name(name), identity);
                            Ok((open_made(fs, target)?, Some(identity)))
                        }
                        Err(_) => Err(Failure::Place(PlaceError::Exists)),
                    }
                },
            )?,
        };
        let name = file_name(&path);
        let target = Entry::new(dir, &name);
        let created = fs.stat_fd(fd.as_fd())?;
        match expected {
            Some(expected) => {
                let ours = match compare(self.mount.identity, expected, None, &created) {
                    Proof::Same => true,
                    Proof::Different => false,
                    Proof::Unproven => created.size == 0,
                };
                if !ours {
                    return Err(Failure::Place(PlaceError::Changed));
                }
            }
            None => {
                tree.made(&path, created.identity);
                self.names.made(&name, created.identity);
            }
        }
        // Takes `fd`, so a failed file is closed before it is removed: NFS
        // renames an open file aside instead.
        let written = (|fd: OwnedFd| -> Result<Option<manifest::Read>, Failure> {
            let parent = &self.parent;
            let Some((snapshot, read)) =
                copy.copy_file(data, &self.mount, &source, fd.as_fd(), parent)?
            else {
                return Ok(None);
            };
            copy.apply_metadata(fd.as_fd(), &created, &snapshot, parent, &self.mount)?;
            copy.sync_written(fd.as_fd(), &self.mount)?;
            fs.close(fd)?;
            copy.settle_times(target, created.identity, &snapshot, &self.mount)?;
            Ok(Some(read))
        })(fd);
        if !matches!(written, Ok(Some(_))) {
            placement::remove_if_unchanged(
                fs,
                target,
                created.identity,
                copy.written_attributes(created.identity),
                AtFlags::empty(),
            );
        }
        let Some(read) = written? else {
            if let Some(created) = &tree.created {
                lock(&created.0).retain(|(made, _)| made != &path);
            }
            if let Some(folder) = &self.recorded_in {
                folder.keep(&self.name, KeptCause::Changed);
            }
            if role == Role::First {
                tree.links.torn(key);
            }
            return Ok(role);
        };
        copy.wrote(created.identity, &read);
        if role == Role::First {
            tree.links
                .written(key, path, created.identity, read.contents.clone());
        }
        if let Some(folder) = &self.recorded_in {
            folder.file(&self.name, read);
        }
        Ok(role)
    }
}

fn open_made(fs: &dyn FsOps, target: Entry<'_>) -> Result<OwnedFd, Failure> {
    fs.open_write(target).map_err(|error| match error {
        // A pipe without a reader took the file's place.
        Errno::NXIO => Failure::Place(PlaceError::Changed),
        error => error.into(),
    })
}

pub(super) struct Finish {
    name: OsString,
    target: Arc<OwnedFd>,
    created: EntryStat,
    parent: EntryStat,
    snapshot: Snapshot,
    path: PathBuf,
    mount: Mount,
    scope: usize,
    completion: Completion,
}

impl Finish {
    fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        let copy = &tree.copy;
        let applied = self.snapshot.apply_to_folder(
            copy.fs,
            self.target.as_fd(),
            &self.created,
            &self.parent,
            &self.mount,
            &copy.refused_xattrs,
        );
        match applied {
            Ok(mode) => {
                tree.links
                    .retime(self.scope, &self.path, self.created.identity, self.snapshot);
                if let Some(mode) = mode {
                    lock(&tree.restrictive).push((
                        self.scope,
                        Restrictive {
                            path: self.path,
                            identity: self.created.identity,
                            mode,
                        },
                    ));
                }
                drop(self.target);
                queue.complete(self.completion);
            }
            Err(error) => queue.fail(Failure::Os(error).into_error(&self.name)),
        }
    }
}

pub(super) struct Merged {
    pub(super) source: (Arc<OwnedFd>, OsString),
    pub(super) target: (Arc<OwnedFd>, OsString, Option<LocalFileIdentity>),
    pub(super) path: PathBuf,
    pub(super) recorded_in: Option<Arc<manifest::Folder>>,
    pub(super) placing: Option<Arc<Placing>>,
    pub(super) parent: Option<MergedNames>,
}

/// Shared by a folder merged into it unless another filesystem is mounted at either side, as a
/// bind mount of the same one shares its rules.
#[derive(Clone, Copy)]
pub(super) struct MergedNames {
    devices: (u64, u64),
    site: Site,
    fixing: bool,
}

impl Merged {
    fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        if let Err(failure) = self.merge(tree, queue) {
            queue.fail(failure.into_error(&self.source.1));
        }
    }

    fn merge(&self, tree: &Arc<Tree>, queue: &Queue) -> Result<(), Failure> {
        let copy = &tree.copy;
        let fs = copy.fs;
        let (source_parent, source_name) = &self.source;
        let source = fs
            .open_dir(Entry::new(&**source_parent, source_name))
            .map_err(|error| open_failure(source_name, error))?;
        tree.source_in(fs, source.as_fd());
        let (target_parent, target_name, expected) = &self.target;
        let target = fs
            .open_dir(Entry::new(&**target_parent, target_name))
            .map_err(|error| open_failure(target_name, error))?;
        let target_stat = fs.stat_fd(target.as_fd())?;
        let mount = copy.mount(target.as_fd());
        if expected.is_some_and(|expected| !mount.may_be(expected, &target_stat)) {
            return Err(Failure::Changed);
        }
        lock(&tree.copies).push(manifest::Published::new(
            target_parent.clone(),
            target_name.clone(),
            target_stat.identity,
        ));
        if copy.durable {
            tree.reach(fs, &mount, target.as_fd(), &self.path)?;
        }
        let source_stat = fs.stat_fd(source.as_fd())?;
        let devices = (source_stat.identity.device, target_stat.identity.device);
        let names = match self.parent {
            Some(parent) if parent.devices == devices => parent,
            _ => {
                let site = copy.site(&mount, target.as_fd());
                MergedNames {
                    devices,
                    site,
                    fixing: tree.crosses(&source_stat, &target_stat, &site),
                }
            }
        };
        let recorded = self
            .recorded_in
            .as_ref()
            .map(|folder| folder.folder(source_name, source_stat.identity));
        // A stage made in the live folder inherits its casefolding.
        let naming = Naming::Decide.decide(fs, mount.family, target.as_fd());
        let folder = copy.names.in_folder(&names.site);
        // As the merge planner, which cannot read a folder's casefolding.
        let children = tree.plan(source.as_fd(), &folder, names.fixing, Key::Folded)?;
        let requested = children
            .iter()
            .map(|(_, _, planned)| planned.target.clone())
            .collect();
        let live = Live {
            names: Names::new(requested, naming, mount.identity, folder),
            site: names.site,
            naming,
            source: Arc::new(source),
            dir: Arc::new(target),
            stat: target_stat,
            mount,
            path: self.path.clone(),
            name: source_name.clone(),
            recorded,
            placing: self.placing.clone(),
        };
        tree.probe_links_in(&mount, live.dir.clone());
        let mut new = Vec::new();
        for (name, kind, planned) in children {
            copy.check_cancelled()?;
            let target_name = &planned.target;
            let existing = match fs.stat(Entry::new(&*live.dir, target_name), false) {
                Ok(existing) => Some(existing),
                Err(Errno::NOENT) => None,
                Err(error) => return Err(Failure::Os(error).named(&name)),
            };
            match existing {
                Some(existing) if existing.is_directory() && kind == FileType::Directory => {
                    planned_rename(copy, &self.path, &name, &planned);
                    queue.enqueue(TreeJob::Merge(
                        tree.clone(),
                        Merged {
                            source: (live.source.clone(), name),
                            target: (
                                live.dir.clone(),
                                target_name.clone(),
                                Some(existing.identity),
                            ),
                            path: self.path.join(target_name),
                            recorded_in: live.recorded.clone(),
                            placing: live.placing.clone(),
                            parent: Some(names),
                        },
                    ));
                }
                // A name taken since the merge was planned gets a number
                // when the child is placed.
                _ => new.push((name, kind, planned)),
            }
        }
        live.add(tree, queue, new)
    }
}

struct Live {
    source: Arc<OwnedFd>,
    dir: Arc<OwnedFd>,
    stat: EntryStat,
    mount: Mount,
    path: PathBuf,
    names: Arc<Names>,
    site: Site,
    naming: Naming,
    name: OsString,
    recorded: Option<Arc<manifest::Folder>>,
    placing: Option<Arc<Placing>>,
}

impl Live {
    fn add(
        &self,
        tree: &Arc<Tree>,
        queue: &Queue,
        mut new: Vec<(OsString, FileType, Planned)>,
    ) -> Result<(), Failure> {
        let mut in_place = Vec::new();
        if placement::can_place(tree.copy.fs, self.dir.as_fd(), true) == Some(false) {
            (in_place, new) = new
                .into_iter()
                .partition(|(_, kind, _)| *kind == FileType::Directory);
        }
        if !new.is_empty() {
            self.stage(tree, queue, new)?;
        }
        for (name, _, planned) in in_place {
            self.copy_in_place(tree, &name, &planned.target)
                .map_err(|failure| failure.named(&name))?;
            planned_rename(&tree.copy, &self.path, &name, &planned);
        }
        Ok(())
    }

    fn stage(
        &self,
        tree: &Arc<Tree>,
        queue: &Queue,
        children: Vec<(OsString, FileType, Planned)>,
    ) -> Result<(), Failure> {
        let fs = tree.copy.fs;
        tree.copy.check_cancelled()?;
        let staged = StagedSibling::create(self.dir.clone(), StageContent::Directory)?;
        let (name, identity) = (staged.name.clone(), staged.identity);
        lock(&tree.stages).push(staged);
        let dir = fs.open_dir(Entry::new(&*self.dir, &name))?;
        if !self.mount.may_be(identity, &fs.stat_fd(dir.as_fd())?) {
            return Err(Failure::Place(PlaceError::Changed));
        }
        let scope = tree.scopes.fetch_add(1, Ordering::Relaxed);
        let made = Made {
            naming: self.naming,
            dir: Arc::new(dir),
            stat: self.stat,
            path: self.path.clone(),
            names: self.names.clone(),
            site: self.site,
            mount: self.mount,
            scope,
            recorded: self.recorded.clone(),
        };
        let place = Place {
            live: self.dir.clone(),
            stage: made.dir.clone(),
            staged: (name, identity),
            path: self.path.clone(),
            names: self.names.clone(),
            mount: self.mount,
            scope,
            name: self.name.clone(),
            placing: self.placing.clone(),
        };
        let group = Group::new(children.len(), TreeJob::Place(tree.clone(), place));
        made.make_all(tree, queue, &self.source, children, &group)
    }

    fn copy_in_place(&self, tree: &Tree, name: &OsStr, target_name: &OsStr) -> Result<(), Failure> {
        let copy = &tree.copy;
        let source = Source {
            parent: self.source.clone(),
            name: name.to_os_string(),
            recorded_in: self.recorded.clone(),
        };
        let expected = copy.fs.stat(source.entry(), false)?.identity;
        if expected.kind != FileType::Directory {
            return Err(Failure::Changed);
        }
        let nested = copy.nested();
        let path = self.path.join(target_name);
        let (path, ownership) = make_numbered(
            copy,
            &self.names,
            &path,
            true,
            RenameCause::Taken,
            |candidate| {
                direct_folder(
                    &nested,
                    &source,
                    expected,
                    &self.dir,
                    candidate,
                    tree.workers,
                )
            },
        )?;
        copy.adopt(&nested, &path);
        if let Some(placing) = &self.placing
            && let Some(name) = path.file_name()
        {
            placing.placed(
                copy.fs,
                (self.dir.as_fd(), name),
                &path,
                &ownership,
                &copy.cancelled,
            );
        }
        if let Some(name) = path.file_name() {
            lock(&tree.copies).push(manifest::Published::proven(
                self.dir.clone(),
                name.to_os_string(),
                ownership,
            ));
        }
        lock(&tree.placed).push(path);
        Ok(())
    }
}

pub(super) struct Place {
    live: Arc<OwnedFd>,
    stage: Arc<OwnedFd>,
    staged: (OsString, LocalFileIdentity),
    path: PathBuf,
    names: Arc<Names>,
    mount: Mount,
    scope: usize,
    name: OsString,
    placing: Option<Arc<Placing>>,
}

impl Place {
    fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        let name = self.name.clone();
        if let Err(failure) = self.place(tree) {
            queue.fail(failure.into_error(&name));
        }
    }

    fn place(self, tree: &Tree) -> Result<(), Failure> {
        let copy = &tree.copy;
        let fs = copy.fs;
        tree.links.make(
            self.scope,
            copy,
            self.stage.as_fd(),
            &self.path,
            &self.mount,
            None,
        )?;
        let mut children = fs.list(self.stage.as_fd())?;
        children.sort_by(|(left, _), (right, _)| left.cmp(right));
        let mut restrictive = tree.take_restrictive(self.scope);
        let mut placed = Vec::new();
        let mut result = Ok(());
        for (name, kind) in &children {
            match self.place_child(copy, name, *kind == FileType::Directory) {
                Ok((path, ownership)) => {
                    let requested = self.path.join(name);
                    if path != requested {
                        copy.relocate(&requested, &path);
                        for folder in &mut restrictive {
                            super::rebase(&mut folder.path, &requested, &path);
                        }
                    }
                    placed.push((path, ownership));
                }
                Err(failure) => {
                    // Either name may hold the child, so the stage is kept.
                    if matches!(failure, Failure::Place(PlaceError::Unconfirmed)) {
                        tree.settle_stage(self.staged.1);
                    }
                    result = Err(failure.named(name));
                    break;
                }
            }
        }
        // Placing a name of a file changes the change time of its others.
        if let Some(placing) = &self.placing {
            for (path, ownership) in &placed {
                if let Some(name) = path.file_name() {
                    placing.placed(
                        fs,
                        (self.live.as_fd(), name),
                        path,
                        ownership,
                        &copy.cancelled,
                    );
                }
            }
        }
        for (path, ownership) in &placed {
            if let Some(name) = path.file_name() {
                lock(&tree.copies).push(manifest::Published::proven(
                    self.live.clone(),
                    name.to_os_string(),
                    ownership.clone(),
                ));
            }
        }
        let placed = placed.into_iter().map(|(path, _)| path).collect::<Vec<_>>();
        let mut placed_folders = restrictive
            .into_iter()
            .filter(|folder| placed.iter().any(|path| folder.path.starts_with(path)))
            .filter_map(|folder| {
                let path = folder.path.strip_prefix(&self.path).ok()?.to_path_buf();
                Some(Restrictive { path, ..folder })
            })
            .collect::<Vec<_>>();
        let tightened = restrictive::tighten(
            fs,
            self.live.as_fd(),
            &mut placed_folders,
            self.mount.identity,
        );
        lock(&tree.placed).extend(placed);
        result?;
        tightened?;
        self.remove_stage(tree)
    }

    fn place_child(
        &self,
        copy: &Arc<Copy>,
        name: &OsStr,
        is_directory: bool,
    ) -> Result<(PathBuf, manifest::Ownership), Failure> {
        copy.check_cancelled()?;
        let fs = copy.fs;
        let staged = Entry::new(&*self.stage, name);
        let identity = fs.stat(staged, false)?.identity;
        let ownership = manifest::Ownership::capture(fs, staged, identity)?;
        let link = (identity.kind == FileType::Symlink)
            .then(|| read_link(fs, staged))
            .transpose()?;
        let path = self.path.join(name);
        make_numbered(
            copy,
            &self.names,
            &path,
            is_directory,
            RenameCause::Taken,
            |candidate| {
                let target = Entry::new(&*self.live, candidate);
                match placement::place_staged(fs, staged, target, identity) {
                    Ok(placement::Placed::RecreatedSymlink) => {
                        Ok(made_again(fs, target, link.as_deref()))
                    }
                    Ok(_) => Ok(ownership.clone()),
                    Err(PlaceError::Unsupported) => self.copy_out(copy, name, identity, candidate),
                    Err(error) => Err(error.into()),
                }
            },
        )
    }

    fn copy_out(
        &self,
        copy: &Arc<Copy>,
        name: &OsStr,
        identity: LocalFileIdentity,
        candidate: &OsStr,
    ) -> Result<manifest::Ownership, Failure> {
        let source = Source {
            parent: self.stage.clone(),
            name: name.to_os_string(),
            recorded_in: None,
        };
        let ownership = from_stage(&copy.uncounted(), &source, identity, &self.live, candidate)?;
        copy.discard(StagedSibling {
            ownership: None,
            parent: self.stage.clone(),
            name: name.to_os_string(),
            identity,
            restrictive: Vec::new(),
        })
        .map_err(|error| Failure::Error(super::stage_left(&error)))?;
        Ok(ownership)
    }

    fn remove_stage(self, tree: &Tree) -> Result<(), Failure> {
        let fs = tree.copy.fs;
        let Self {
            live,
            stage: opened,
            staged: (name, identity),
            ..
        } = self;
        let stage = Entry::new(&*live, &name);
        let held = fs.stat_fd(opened.as_fd()).ok().map(|held| held.identity);
        let unchanged = || {
            fs.stat(stage, false)
                .is_ok_and(|current| held == Some(current.identity))
        };
        let ours = unchanged();
        drop(opened);
        let removed = if ours {
            match tree.copy.remove_folder(stage, &unchanged) {
                Ok(()) | Err(Refused::Os(Errno::NOENT) | Refused::Changed(_)) => Ok(()),
                Err(refused) => Err(refused.errno()),
            }
        } else {
            Ok(())
        };
        tree.settle_stage(identity);
        removed
            .map_err(|error| Failure::Error(super::stage_left(&io_error(os_error_detail(error)))))
    }
}
