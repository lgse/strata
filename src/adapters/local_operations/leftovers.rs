// SPDX-License-Identifier: MIT

use std::{
    cell::OnceCell,
    collections::HashSet,
    ffi::{OsStr, OsString},
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gtk::{gio, glib, prelude::*};
use rustix::{
    fs::{AtFlags, FileType, Mode},
    io::Errno,
};

use super::{
    FailedAction, UNSUPPORTED_REASON, cancelled_event,
    copy_engine::now,
    deferred_close::{self, Pause, Wait},
    fs_ops::{Entry, EntryStat, FsOps, fs},
    holders, item_error,
    mount_profile::{known_mount, name_limit},
    names::numbered_names,
    native_fs::{open_local_parent_directory, os_error_detail},
    operation_error_summary, operation_handle,
    placement::{self, PlaceError},
    placement_reason,
    release::{self, ActiveOperation},
    run_blocking,
};
use crate::{
    adapters::volume::{
        Family, SillyNames,
        mount_lanes::{self, Class, Progress},
    },
    model::{
        Location,
        hidden_names::{Leftover, classify, foreign_leftover},
    },
    services::{
        Holder, KeptCause, KeptItem, LeftoverInside, LoadHandle, OperationEvent, RecoverRequest,
        RenameCause, RenamedItem, ReviewedLeftover, TransferReport,
    },
};

const LINGERS: Duration = Duration::from_secs(2);
const PROBES_STALE_AFTER: Duration = Duration::from_secs(60 * 60);
const REVIEWED_ENTRIES: usize = 1_000;

static SWEPT: LazyLock<Mutex<HashSet<(PathBuf, OsString)>>> = LazyLock::new(Mutex::default);

/// Shared by every job of one deletion: when its leftover wait began and how many
/// entries its workers have removed.
#[derive(Clone, Default)]
pub(crate) struct Deadline {
    started: Arc<OnceLock<SystemTime>>,
    removed: Arc<AtomicUsize>,
}

impl Deadline {
    pub(super) fn wait(&self, budget: Duration) -> Wait {
        Wait::since(*self.started.get_or_init(now), budget)
    }

    pub(super) fn note_removed(&self) {
        self.removed.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn removed(&self) -> usize {
        self.removed.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Refused {
    Os(Errno),
    StillOpen(Errno),
    Stopped(Errno),
    Changed(Errno),
}

impl Refused {
    pub(super) fn errno(self) -> Errno {
        match self {
            Self::Os(errno)
            | Self::StillOpen(errno)
            | Self::Stopped(errno)
            | Self::Changed(errno) => errno,
        }
    }
}

pub(super) fn remove_folder(
    fs: &dyn FsOps,
    at: Entry<'_>,
    deadline: &Deadline,
    stopped: &dyn Fn() -> bool,
    unchanged: &dyn Fn() -> bool,
) -> Result<(), Refused> {
    let mut wait = None;
    let mut at_once = false;
    loop {
        let errno = match fs.unlink(at, AtFlags::REMOVEDIR) {
            Ok(()) => return Ok(()),
            Err(errno) => errno,
        };
        let Some(held) = held_only(fs, at, errno) else {
            return Err(Refused::Os(errno));
        };
        at_once = held.emptied && !at_once;
        if !at_once {
            let wait = wait.get_or_insert_with(|| deadline.wait(held.budget));
            match wait.pause(stopped) {
                Pause::Retry => {}
                Pause::Spent => return Err(Refused::StillOpen(errno)),
                Pause::Cancelled => return Err(Refused::Stopped(errno)),
            }
        }
        if !unchanged() {
            return Err(Refused::Changed(errno));
        }
    }
}

pub(super) fn remove_held_folder(
    fs: &dyn FsOps,
    at: Entry<'_>,
    held: BorrowedFd<'_>,
    deadline: &Deadline,
    stopped: &dyn Fn() -> bool,
) -> Result<(), Refused> {
    let unchanged = || match (fs.stat(at, false), fs.stat_fd(held)) {
        (Ok(named), Ok(held)) => named.identity == held.identity,
        _ => false,
    };
    if !unchanged() {
        return Err(Refused::Changed(Errno::NOENT));
    }
    remove_folder(fs, at, deadline, stopped, &unchanged)
}

pub(super) fn kept_open(fs: &dyn FsOps, folder: Entry<'_>, errno: Errno) -> KeptCause {
    match holder(fs, folder) {
        None if errno == Errno::ACCESS => KeptCause::Undeletable(errno.raw_os_error()),
        holder => KeptCause::StillOpen(holder),
    }
}

pub(super) fn holder(fs: &dyn FsOps, at: Entry<'_>) -> Option<Holder> {
    let leftovers: Vec<_> = fs
        .names(at)
        .unwrap_or_default()
        .into_iter()
        .filter(|name| foreign_leftover(name).is_some())
        .collect();
    holders::find(fs, at, &leftovers)
}

pub(super) fn still_open(fs: &dyn FsOps, at: Entry<'_>) -> Option<String> {
    holders::held_inside(holder(fs, at).as_ref())
        .or_else(|| holders::held_elsewhere(known_mount(fs, at.dir).1.silly_names))
}

pub(super) fn is_leftover(fs: &dyn FsOps, dir: BorrowedFd<'_>, name: &OsStr) -> bool {
    foreign_leftover(name).is_some_and(|family| {
        known_mount(fs, dir).1.silly_names == Some(SillyNames::Renamed(family))
    })
}

struct Held {
    emptied: bool,
    budget: Duration,
}

fn held_only(fs: &dyn FsOps, at: Entry<'_>, errno: Errno) -> Option<Held> {
    if !matches!(errno, Errno::NOTEMPTY | Errno::EXIST | Errno::ACCESS) {
        return None;
    }
    let names = fs.names(at).ok()?;
    if names.iter().any(|name| foreign_leftover(name).is_none()) {
        return None;
    }
    let (_, capabilities) = known_mount(fs, at.dir);
    let only_leftovers = match (capabilities.silly_names?, errno) {
        (SillyNames::Renamed(family), Errno::NOTEMPTY | Errno::EXIST) => names
            .iter()
            .all(|name| foreign_leftover(name) == Some(family)),
        (SillyNames::Unlisted, Errno::NOTEMPTY | Errno::EXIST | Errno::ACCESS) => names.is_empty(),
        _ => false,
    };
    only_leftovers.then(|| Held {
        emptied: names.is_empty() && errno != Errno::ACCESS,
        budget: capabilities
            .deferred_close
            .map_or(LINGERS, deferred_close::closing),
    })
}

pub(super) fn start_recovery(
    request: RecoverRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let operation_cancellable = cancellable.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let container = request.container;
        let mut affected: HashSet<_> = std::iter::once(container.clone())
            .chain(container.parent())
            .collect();
        let Some(path) = container.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: crate::i18n::tr(UNSUPPORTED_REASON),
                password_failure: None,
            });
            return;
        };
        let operation = ActiveOperation::begin(release::parents([&path]));
        operation
            .release(vec![path.clone()], &operation_cancellable)
            .await;
        if operation_cancellable.is_cancelled() {
            emit(cancelled_event(
                request.id,
                Vec::new(),
                Vec::new(),
                vec![container],
                affected,
            ));
            return;
        }
        let stop = operation_cancellable.clone();
        let recovered = run_blocking(move || Ok(recover(fs(), &path, &|| stop.is_cancelled())))
            .await
            .unwrap_or_else(|error| Err(crate::services::gio_error_message(&error)));
        let recovered = match recovered {
            Ok(recovered) => recovered,
            Err(message) => {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message,
                    password_failure: None,
                });
                return;
            }
        };
        affected.extend(recovered.restored.iter().filter_map(Location::parent));
        if !recovered.report.is_empty() {
            emit(OperationEvent::ItemsReported {
                request_id: request.id,
                report: recovered.report,
            });
        }
        let (failed, errors): (Vec<_>, Vec<_>) = recovered.failed.into_iter().unzip();
        let left = if recovered.removed {
            vec![container]
        } else {
            Vec::new()
        };
        if !recovered.not_attempted.is_empty() {
            emit(cancelled_event(
                request.id,
                recovered.restored,
                failed,
                recovered.not_attempted,
                affected,
            ));
        } else if errors.is_empty() {
            emit(OperationEvent::Restored {
                request_id: request.id,
                locations: left,
                restored: recovered.restored,
            });
        } else {
            emit(OperationEvent::RestoreCompletedWithErrors {
                request_id: request.id,
                restored_locations: left,
                restored: recovered.restored,
                message: operation_error_summary(&errors, FailedAction::Restore),
            });
        }
        drop(operation);
    });
    operation_handle(cancellable)
}

#[derive(Default)]
pub(super) struct Recovered {
    pub restored: Vec<Location>,
    pub report: TransferReport,
    pub failed: Vec<(Location, String)>,
    pub not_attempted: Vec<Location>,
    pub removed: bool,
}

pub(super) fn recover(
    fs: &dyn FsOps,
    container: &Path,
    stopped: &dyn Fn() -> bool,
) -> Result<Recovered, String> {
    let invalid = || crate::i18n::tr("Invalid local filesystem target");
    let (Some(root), Some(name)) = (container.parent(), container.file_name()) else {
        return Err(invalid());
    };
    let mirrors = match classify(name) {
        Some(Leftover::Originals) => false,
        Some(Leftover::MergeOriginals) => true,
        _ => return Err(invalid()),
    };
    let folder = open_local_parent_directory(root)?;
    let failed = |reason: String| item_error(&name.to_string_lossy(), reason);
    let held = placement::open_beneath(fs, folder.as_fd(), Path::new(name))
        .map_err(|error| failed(placement_reason(error)))?;
    let names = listed(fs, &held).map_err(|error| failed(os_error_detail(error)))?;
    let mut recovery = Recovery {
        fs,
        stopped,
        root,
        container,
        deadline: Deadline::default(),
        recovered: Recovered::default(),
    };
    let destination = Destination::Folder(folder.as_fd());
    recovery.empty(&held, names, &destination, Path::new(""), mirrors);
    if !stopped() {
        recovery.recovered.removed =
            recovery.remove(Entry::new(&folder, name), &held, Path::new(""), 0);
    }
    Ok(recovery.recovered)
}

enum Destination<'p> {
    Folder(BorrowedFd<'p>),
    Mirrored {
        within: &'p Destination<'p>,
        name: &'p OsStr,
        relative: &'p Path,
        opened: OnceCell<Option<OwnedFd>>,
    },
}

impl Destination<'_> {
    fn open(&self, recovery: &mut Recovery<'_>) -> Option<BorrowedFd<'_>> {
        match self {
            Self::Folder(dir) => Some(*dir),
            Self::Mirrored {
                within,
                name,
                relative,
                opened,
            } => opened
                .get_or_init(|| {
                    let dir = within.open(recovery)?;
                    match folder(recovery.fs, Entry { dir, name }) {
                        Ok(opened) => Some(opened),
                        Err(error) => {
                            recovery.failed(relative, error, dir);
                            None
                        }
                    }
                })
                .as_ref()
                .map(AsFd::as_fd),
        }
    }
}

struct Recovery<'a> {
    fs: &'a dyn FsOps,
    stopped: &'a dyn Fn() -> bool,
    root: &'a Path,
    container: &'a Path,
    deadline: Deadline,
    recovered: Recovered,
}

impl Recovery<'_> {
    fn empty(
        &mut self,
        held: &OwnedFd,
        names: Vec<OsString>,
        into: &Destination<'_>,
        relative: &Path,
        mirrors: bool,
    ) {
        let fs = self.fs;
        for name in names {
            let path = relative.join(&name);
            if (self.stopped)() {
                self.recovered
                    .not_attempted
                    .push(Location::local(self.inside(&path)));
                continue;
            }
            if is_leftover(fs, held.as_fd(), &name) {
                continue;
            }
            let from = Entry::new(held, &name);
            let stat = match fs.stat(from, false) {
                Ok(stat) => stat,
                Err(Errno::NOENT) => continue,
                Err(error) => {
                    self.failed(&path, PlaceError::Io(error), held.as_fd());
                    continue;
                }
            };
            if mirrors && stat.is_directory() {
                self.mirror(from, into, &path);
            } else if stat.is_directory() && only_leftovers(fs, from) {
                let cause = KeptCause::StillOpen(holder(fs, from));
                self.kept(&path, cause);
            } else if let Some(dir) = into.open(self) {
                self.put(from, &stat, Entry { dir, name: &name }, relative);
            }
        }
    }

    fn mirror(&mut self, mirror: Entry<'_>, into: &Destination<'_>, relative: &Path) {
        let opened = placement::open_beneath(self.fs, mirror.dir, Path::new(mirror.name))
            .and_then(|held| Ok((listed(self.fs, &held).map_err(PlaceError::Io)?, held)));
        let (names, held) = match opened {
            Ok(opened) => opened,
            Err(error) => return self.failed(relative, error, mirror.dir),
        };
        let reported = self.reported();
        let destination = Destination::Mirrored {
            within: into,
            name: mirror.name,
            relative,
            opened: OnceCell::new(),
        };
        self.empty(&held, names, &destination, relative, true);
        if !(self.stopped)() {
            self.remove(mirror, &held, relative, reported);
        }
    }

    fn put(&mut self, from: Entry<'_>, stat: &EntryStat, to: Entry<'_>, relative: &Path) {
        let fs = self.fs;
        let requested = relative.join(to.name);
        let place = |name: &OsStr| {
            placement::place_back(
                fs,
                from,
                Entry { dir: to.dir, name },
                stat.identity,
                stat.kept_attributes(),
            )
        };
        match place(to.name) {
            Ok(_) => return self.restored(&requested),
            Err(PlaceError::Exists) if same_inode(fs, from, to) => {
                return match fs.unlink(from, AtFlags::empty()) {
                    Ok(()) | Err(Errno::NOENT) => self.restored(&requested),
                    Err(error) => self.failed(&requested, PlaceError::Io(error), to.dir),
                };
            }
            Err(PlaceError::Exists) => {}
            Err(error) => return self.failed(&requested, error, to.dir),
        }
        let limit = name_limit(fs, to.dir);
        for name in numbered_names(to.name, stat.is_directory(), limit) {
            match place(&name) {
                Ok(_) => {
                    let saved = relative.join(&name);
                    self.recovered.report.renamed.push(RenamedItem {
                        requested: Location::local(self.root.join(&requested)),
                        saved_as: Location::local(self.root.join(&saved)),
                        cause: RenameCause::Taken,
                    });
                    return self.restored(&saved);
                }
                Err(PlaceError::Exists) => {}
                Err(error) => return self.failed(&requested, error, to.dir),
            }
        }
        self.failed(&requested, PlaceError::Exists, to.dir);
    }

    fn remove(
        &mut self,
        folder: Entry<'_>,
        held: &OwnedFd,
        relative: &Path,
        reported: usize,
    ) -> bool {
        let fs = self.fs;
        let removed = remove_held_folder(fs, folder, held.as_fd(), &self.deadline, self.stopped);
        let refused = match removed {
            Ok(()) | Err(Refused::Os(Errno::NOENT)) => return true,
            Err(Refused::Os(Errno::NOTEMPTY | Errno::EXIST) | Refused::Changed(_)) => return false,
            Err(Refused::Stopped(_)) => return false,
            Err(refused) => refused,
        };
        let cause = match refused {
            Refused::Os(_) if self.reported() != reported => return false,
            Refused::Os(errno) if self.needs_check(errno, folder.dir) => {
                self.failed(relative, PlaceError::Io(errno), folder.dir);
                return false;
            }
            Refused::StillOpen(errno) => kept_open(fs, folder, errno),
            refused => KeptCause::Undeletable(refused.errno().raw_os_error()),
        };
        self.kept(relative, cause);
        false
    }

    fn restored(&mut self, relative: &Path) {
        self.recovered
            .restored
            .push(Location::local(self.root.join(relative)));
    }

    fn reported(&self) -> usize {
        let recovered = &self.recovered;
        recovered.failed.len() + recovered.report.kept.len() + recovered.not_attempted.len()
    }

    fn inside(&self, relative: &Path) -> PathBuf {
        if relative.as_os_str().is_empty() {
            self.container.to_path_buf()
        } else {
            self.container.join(relative)
        }
    }

    fn kept(&mut self, relative: &Path, cause: KeptCause) {
        self.recovered.report.kept.push(KeptItem {
            location: Location::local(self.inside(relative)),
            cause,
        });
    }

    fn needs_check(&self, error: Errno, dir: BorrowedFd<'_>) -> bool {
        error == Errno::ROFS && known_mount(self.fs, dir).1.family == Some(Family::Ntfs3)
    }

    fn failed(&mut self, relative: &Path, error: PlaceError, dir: BorrowedFd<'_>) {
        let reason = match error {
            PlaceError::Io(error) if self.needs_check(error, dir) => crate::i18n::tr(
                "The drive is read-only. It may need to be checked for errors after it was unplugged without being ejected.",
            ),
            error => placement_reason(error),
        };
        let location = self.inside(relative);
        let shown = if relative.as_os_str().is_empty() {
            self.container.file_name().unwrap_or_default()
        } else {
            relative.as_os_str()
        };
        self.recovered.failed.push((
            Location::local(location),
            item_error(&shown.to_string_lossy(), reason),
        ));
    }
}

fn folder(fs: &dyn FsOps, at: Entry<'_>) -> Result<OwnedFd, PlaceError> {
    match placement::open_beneath(fs, at.dir, Path::new(at.name)) {
        Err(PlaceError::Io(Errno::NOENT)) => {}
        opened => return opened,
    }
    match fs.mkdir(at, Mode::from_raw_mode(0o777)) {
        Ok(()) | Err(Errno::EXIST) => placement::open_beneath(fs, at.dir, Path::new(at.name)),
        Err(error) => Err(placement::creation_error(error)),
    }
}

fn listed(fs: &dyn FsOps, held: &OwnedFd) -> Result<Vec<OsString>, Errno> {
    let mut names = fs.names(Entry::new(held, OsStr::new(".")))?;
    names.sort();
    Ok(names)
}

fn same_inode(fs: &dyn FsOps, aside: Entry<'_>, named: Entry<'_>) -> bool {
    let (Ok(aside), Ok(named)) = (fs.stat(aside, false), fs.stat(named, false)) else {
        return false;
    };
    !aside.is_directory()
        && aside.nlink > 1
        && aside.identity == named.identity
        && (aside.nlink, aside.size, aside.mtime, aside.ctime)
            == (named.nlink, named.size, named.mtime, named.ctime)
}

fn only_leftovers(fs: &dyn FsOps, folder: Entry<'_>) -> bool {
    let Ok(held) = placement::open_beneath(fs, folder.dir, Path::new(folder.name)) else {
        return false;
    };
    let Ok(names) = listed(fs, &held) else {
        return false;
    };
    !names.is_empty()
        && names.iter().all(|name| {
            let entry = Entry::new(&held, name);
            is_leftover(fs, held.as_fd(), name)
                || fs.stat(entry, false).is_ok_and(|stat| stat.is_directory())
                    && only_leftovers(fs, entry)
        })
}

pub(super) fn start_sweep(folder: &Location, names: &[OsString]) {
    let Some(folder) = folder.native_path().map(Path::to_path_buf) else {
        return;
    };
    let probes: Vec<_> = names
        .iter()
        .filter(|name| classify(name) == Some(Leftover::Probe))
        .cloned()
        .collect();
    if probes.is_empty() {
        return;
    }
    let lane = folder.clone();
    drop(mount_lanes::run(
        &lane,
        Class::Background,
        move |progress| {
            sweep_probes(fs(), &folder, &probes, progress);
        },
    ));
}

pub(super) fn sweep_probes(fs: &dyn FsOps, folder: &Path, names: &[OsString], progress: &Progress) {
    let claimed: Vec<_> = {
        let mut swept = SWEPT.lock().unwrap_or_else(|poison| poison.into_inner());
        names
            .iter()
            .filter(|name| {
                classify(name) == Some(Leftover::Probe)
                    && swept.insert((folder.to_path_buf(), (*name).clone()))
            })
            .collect()
    };
    if claimed.is_empty() {
        return;
    }
    let Ok(dir) = open_local_parent_directory(folder) else {
        return;
    };
    progress.beat();
    let now = now();
    for name in claimed {
        let entry = Entry::new(&dir, name);
        let stat = fs.stat(entry, false);
        progress.answered(&stat);
        let Ok(stat) = stat else {
            continue;
        };
        let flags = match stat.kind {
            FileType::RegularFile if stat.size == 0 => AtFlags::empty(),
            FileType::Directory => AtFlags::REMOVEDIR,
            _ => continue,
        };
        let (seconds, nanoseconds) = stat.mtime;
        let stale = u64::try_from(seconds)
            .ok()
            .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::new(seconds, nanoseconds)))
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > PROBES_STALE_AFTER);
        if stale {
            progress.answered(&fs.unlink(entry, flags));
        }
    }
}

pub(super) fn start_review(
    folder: &Location,
    names: Vec<OsString>,
    done: Box<dyn FnOnce(Vec<ReviewedLeftover>)>,
) -> LoadHandle {
    let Some(folder) = folder.native_path().map(Path::to_path_buf) else {
        done(Vec::new());
        return LoadHandle::new(|| {});
    };
    let lane = folder.clone();
    let reviewed = mount_lanes::run(&lane, Class::Interactive, move |progress| {
        review(fs(), &folder, &names, progress)
    });
    let task = glib::MainContext::default().spawn_local(async move {
        done(reviewed.await.unwrap_or_default());
    });
    LoadHandle::new(move || task.abort())
}

pub(super) fn review(
    fs: &dyn FsOps,
    folder: &Path,
    names: &[OsString],
    progress: &Progress,
) -> Vec<ReviewedLeftover> {
    let reviewed = |name: &OsString, modified, inside| ReviewedLeftover {
        name: name.clone(),
        modified,
        inside,
    };
    let dir = match open_local_parent_directory(folder) {
        Ok(dir) => dir,
        Err(error) => {
            return names
                .iter()
                .map(|name| reviewed(name, None, LeftoverInside::Unreadable(error.clone())))
                .collect();
        }
    };
    progress.beat();
    names
        .iter()
        .map(|name| {
            let entry = Entry::new(&dir, name);
            let stat = fs.stat(entry, false);
            progress.answered(&stat);
            let stat = match stat {
                Ok(stat) => stat,
                Err(Errno::NOENT) => return reviewed(name, None, LeftoverInside::Gone),
                Err(error) => {
                    let message = crate::services::io_error_message(&error.into());
                    return reviewed(name, None, LeftoverInside::Unreadable(message));
                }
            };
            let inside = match classify(name) {
                Some(Leftover::Originals) if stat.is_directory() => {
                    contents(fs, entry, false, progress)
                }
                Some(Leftover::MergeOriginals) if stat.is_directory() => {
                    contents(fs, entry, true, progress)
                }
                _ => LeftoverInside::Item,
            };
            reviewed(name, Some(stat.mtime.0), inside)
        })
        .collect()
}

fn contents(
    fs: &dyn FsOps,
    container: Entry<'_>,
    mirrors: bool,
    progress: &Progress,
) -> LeftoverInside {
    let held = match placement::open_beneath(fs, container.dir, Path::new(container.name)) {
        Ok(held) => held,
        Err(error) => return LeftoverInside::Unreadable(placement_reason(error)),
    };
    let mut shown = Vec::new();
    let mut more = 0;
    let mut held_open = false;
    let mut folders = vec![(held, PathBuf::new())];
    while let Some((folder, relative)) = folders.pop() {
        let names = listed(fs, &folder);
        progress.answered(&names);
        let names = match names {
            Ok(names) => names,
            Err(error) => return LeftoverInside::Unreadable(os_error_detail(error)),
        };
        for name in names {
            if is_leftover(fs, folder.as_fd(), &name) {
                held_open = true;
                continue;
            }
            let path = relative.join(&name);
            if shown.len() >= REVIEWED_ENTRIES {
                more += 1;
                continue;
            }
            let entry = Entry::new(&folder, &name);
            if mirrors && fs.stat(entry, false).is_ok_and(|stat| stat.is_directory()) {
                match placement::open_beneath(fs, folder.as_fd(), Path::new(&name)) {
                    Ok(mirror) => folders.push((mirror, path)),
                    Err(error) => return LeftoverInside::Unreadable(placement_reason(error)),
                }
                continue;
            }
            shown.push(path);
        }
    }
    if shown.is_empty() && more == 0 && held_open {
        let reason = still_open(fs, container).unwrap_or_else(|| holders::held(None));
        return LeftoverInside::StillOpen(reason);
    }
    shown.sort();
    LeftoverInside::Items { names: shown, more }
}
