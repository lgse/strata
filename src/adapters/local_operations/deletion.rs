// SPDX-License-Identifier: MIT

use super::fs_ops::{Entry, EntryStat, fs};
use crate::{
    adapters::{
        gio_close, gio_file_for_location,
        trash_support::{LeftoverWatch, SystemTrash, TrashEnvironment, TrashError, TrashFailure},
    },
    model::{FileEntry, Location, hidden_names},
    services::{OperationEvent, OperationRequestId},
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::Cell,
    collections::HashSet,
    ffi::{OsStr, OsString},
    future::Future,
    io,
    os::fd::{AsFd, OwnedFd},
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    thread,
};

use super::{
    MAX_LOCAL_IO_WORKERS, await_cancellable, cancelled_event, deletion_error_message,
    deletion_error_summary, deletion_undone_when_cancelled, fs_ops, io_error,
    is_trash_unsupported_failure, join_local_tasks,
    leftovers::{self, Deadline, Refused},
    mount_profile::identity_at,
    native_fs::{
        self, Recorded, local_directory_children, open_local_parent_directory, os_error_detail,
        retry_local_open,
    },
    next_children,
};

use super::{
    run_cancellable, translated_io_error,
    work_queue::{self, Group, Job, WorkQueue},
};

pub(super) fn permanently_delete(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if directory {
            let enumerator = gio_close::enumerate_children(
                &file,
                "standard::name,standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                Some(&cancellable),
            )
            .await?;
            loop {
                let children = await_cancellable(
                    &*enumerator,
                    &cancellable,
                    |enumerator, cancellable, result| {
                        enumerator.next_files_async(
                            64,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    },
                )
                .await?;
                if children.is_empty() {
                    break;
                }
                for child in children {
                    permanently_delete(
                        file.child(child.name()),
                        child.file_type() == gio::FileType::Directory,
                        cancellable.clone(),
                    )
                    .await?;
                }
            }
        }
        await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.delete_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
                result.resolve(output)
            });
        })
        .await
    })
}

pub(super) fn permanently_delete_keeping(
    file: gio::File,
    kept: Rc<Vec<gio::File>>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if kept.iter().any(|kept| kept.equal(&file)) {
            return Ok(());
        }
        if !kept.iter().any(|kept| kept.has_prefix(&file)) {
            return permanently_delete(file, true, cancellable).await;
        }
        let enumerator = gio_close::enumerate_children(
            &file,
            "standard::name,standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(&cancellable),
        )
        .await?;
        loop {
            let children = next_children(&enumerator, &cancellable).await?;
            if children.is_empty() {
                return Ok(());
            }
            for child in children {
                let child_file = file.child(child.name());
                if child.file_type() == gio::FileType::Directory {
                    permanently_delete_keeping(child_file, kept.clone(), cancellable.clone())
                        .await?;
                } else if !kept.iter().any(|kept| kept.equal(&child_file)) {
                    permanently_delete(child_file, false, cancellable.clone()).await?;
                }
            }
        }
    })
}

pub(super) enum LocalDeleteStep {
    Removed,
    Leftover,
    Directory {
        handle: OwnedFd,
        children: Vec<OsString>,
    },
}

pub(super) fn ensure_expected_local_identity<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
    current: &EntryStat,
    expected: Option<Recorded>,
) -> Result<(), String> {
    let unchanged = |expected: Recorded| {
        native_fs::removable(
            expected,
            current,
            || identity_at(fs(), parent.as_fd()),
            hidden_names::classify(name).is_some(),
        )
    };
    if expected.is_some_and(|expected| !unchanged(expected)) {
        return Err(rust_i18n::t!(
            "%{name} changed while the operation was in progress",
            name = name.to_string_lossy()
        )
        .into_owned());
    }
    Ok(())
}

/// Check the name too: an open descriptor can outlive a rename.
pub(super) fn ensure_local_delete_target_unchanged<ParentFd: AsFd, TargetFd: AsFd>(
    parent: &ParentFd,
    name: &OsStr,
    target: &TargetFd,
) -> Result<(), String> {
    let named = fs()
        .stat(Entry::new(parent, name), false)
        .map_err(|error| {
            rust_i18n::t!(
                "%{name} changed while it was being deleted: %{error}",
                name = name.to_string_lossy(),
                error = os_error_detail(error)
            )
            .into_owned()
        })?;
    let opened = fs().stat_fd(target.as_fd()).map_err(|error| {
        rust_i18n::t!(
            "Could not recheck %{name}: %{error}",
            name = name.to_string_lossy(),
            error = os_error_detail(error)
        )
        .into_owned()
    })?;
    if named.identity != opened.identity {
        return Err(rust_i18n::t!(
            "%{name} changed while it was being deleted",
            name = name.to_string_lossy()
        )
        .into_owned());
    }
    Ok(())
}

pub(super) fn open_local_delete_target<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
    expected: Option<Recorded>,
    leftovers_stay: bool,
) -> Result<LocalDeleteStep, String> {
    let is_leftover = || leftovers_stay && leftovers::is_leftover(fs(), parent.as_fd(), name);
    let stat = match fs().stat(Entry::new(parent, name), false) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) if is_leftover() => return Ok(LocalDeleteStep::Leftover),
        Err(error) => {
            return Err(rust_i18n::t!(
                "Could not inspect %{name}: %{error}",
                name = name.to_string_lossy(),
                error = os_error_detail(error)
            )
            .into_owned());
        }
    };
    ensure_expected_local_identity(parent, name, &stat, expected)?;
    if !stat.is_directory() {
        if is_leftover() {
            return Ok(LocalDeleteStep::Leftover);
        }
        fs().unlink(Entry::new(parent, name), rustix::fs::AtFlags::empty())
            .map_err(|error| {
                rust_i18n::t!(
                    "Could not delete “%{name}”: %{error}",
                    name = name.to_string_lossy(),
                    error = os_error_detail(error)
                )
                .into_owned()
            })?;
        return Ok(LocalDeleteStep::Removed);
    }
    // Reject symlinks swapped in after the type check, including intermediate components.
    let handle = retry_local_open(|| {
        fs().open_at(
            Entry::new(parent, name),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            Some(
                rustix::fs::ResolveFlags::BENEATH
                    | rustix::fs::ResolveFlags::NO_SYMLINKS
                    | rustix::fs::ResolveFlags::NO_MAGICLINKS,
            ),
        )
    })
    .map_err(|error| {
        rust_i18n::t!(
            "%{name} changed while it was being deleted: %{error}",
            name = name.to_string_lossy(),
            error = os_error_detail(error)
        )
        .into_owned()
    })?;
    let opened = fs().stat_fd(handle.as_fd()).map_err(|error| {
        rust_i18n::t!(
            "Could not recheck %{name}: %{error}",
            name = name.to_string_lossy(),
            error = os_error_detail(error)
        )
        .into_owned()
    })?;
    ensure_expected_local_identity(parent, name, &opened, expected)?;
    let children = local_directory_children(&handle)?;
    Ok(LocalDeleteStep::Directory { handle, children })
}

pub(super) fn cancelled_local_delete() -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Cancelled, "Delete cancelled")
}

pub(super) async fn run_local_delete_step<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, glib::Error> {
    gio::spawn_blocking(work)
        .await
        .map_err(|_| translated_io_error("The deletion stopped unexpectedly"))?
        .map_err(io_error)
}

#[derive(Clone)]
pub(super) struct PinnedEntry {
    pub(super) parent: Arc<OwnedFd>,
    pub(super) name: OsString,
    pub(super) identity: Option<Recorded>,
}

pub(super) struct LocalDeleteGuard {
    pub(super) parent: Arc<OwnedFd>,
    pub(super) name: OsString,
    pub(super) handle: Arc<OwnedFd>,
}

pub(super) type LocalDeleteQueue = WorkQueue<LocalDeleteJob>;

pub(super) enum LocalDeleteJob {
    Entry {
        parent: Arc<OwnedFd>,
        name: OsString,
        expected: Option<Recorded>,
        guard: Option<Arc<LocalDeleteGuard>>,
        completion: Option<Arc<Group<LocalDeleteJob>>>,
        deadline: Deadline,
    },
    RemoveDirectory {
        parent: Arc<OwnedFd>,
        name: OsString,
        handle: Arc<OwnedFd>,
        completion: Option<Arc<Group<LocalDeleteJob>>>,
        deadline: Deadline,
    },
}

impl Job for LocalDeleteJob {
    type Error = String;

    fn run(self, queue: &LocalDeleteQueue) {
        process_local_delete_job(queue, self);
    }

    fn cancelled() -> String {
        "Delete cancelled".to_owned()
    }

    fn panicked() -> String {
        "Delete worker panicked".to_owned()
    }

    fn not_started(error: io::Error) -> String {
        format!("Could not start delete worker: {error}")
    }
}

pub(super) fn process_local_delete_job(queue: &LocalDeleteQueue, job: LocalDeleteJob) {
    match job {
        LocalDeleteJob::Entry {
            parent,
            name,
            expected,
            guard,
            completion,
            deadline,
        } => {
            if queue.is_stopped() {
                return;
            }
            let in_deleted_folder = guard.is_some();
            if let Some(guard) = guard
                && let Err(error) = ensure_local_delete_target_unchanged(
                    guard.parent.as_ref(),
                    &guard.name,
                    guard.handle.as_ref(),
                )
            {
                queue.fail(error);
                return;
            }
            let step =
                match open_local_delete_target(parent.as_ref(), &name, expected, in_deleted_folder)
                {
                    Ok(step) => step,
                    Err(error) => {
                        queue.fail(error);
                        return;
                    }
                };
            match step {
                LocalDeleteStep::Removed => {
                    deadline.note_removed();
                    queue.complete(completion);
                }
                LocalDeleteStep::Leftover => queue.complete(completion),
                LocalDeleteStep::Directory { handle, children } => {
                    let handle = Arc::new(handle);
                    if children.is_empty() {
                        remove_local_delete_directory(
                            queue, parent, name, handle, completion, &deadline,
                        );
                        return;
                    }
                    let guard = Arc::new(LocalDeleteGuard {
                        parent: parent.clone(),
                        name: name.clone(),
                        handle: handle.clone(),
                    });
                    let group = Group::new(
                        children.len(),
                        LocalDeleteJob::RemoveDirectory {
                            parent: parent.clone(),
                            name,
                            handle: handle.clone(),
                            completion,
                            deadline: deadline.clone(),
                        },
                    );
                    for child in children {
                        queue.enqueue(LocalDeleteJob::Entry {
                            parent: handle.clone(),
                            name: child,
                            expected: None,
                            guard: Some(guard.clone()),
                            completion: Some(group.clone()),
                            deadline: deadline.clone(),
                        });
                    }
                }
            }
        }
        LocalDeleteJob::RemoveDirectory {
            parent,
            name,
            handle,
            completion,
            deadline,
        } => remove_local_delete_directory(queue, parent, name, handle, completion, &deadline),
    }
}

pub(super) fn remove_local_delete_directory(
    queue: &LocalDeleteQueue,
    parent: Arc<OwnedFd>,
    name: OsString,
    handle: Arc<OwnedFd>,
    completion: Option<Arc<Group<LocalDeleteJob>>>,
    deadline: &Deadline,
) {
    if queue.is_stopped() {
        return;
    }
    if let Err(error) =
        ensure_local_delete_target_unchanged(parent.as_ref(), &name, handle.as_ref())
    {
        queue.fail(error);
        return;
    }
    let changed = Cell::new(None);
    let unchanged = || {
        ensure_local_delete_target_unchanged(parent.as_ref(), &name, handle.as_ref())
            .map_err(|error| changed.set(Some(error)))
            .is_ok()
    };
    let folder = Entry::new(parent.as_ref(), &name);
    match leftovers::remove_folder(fs(), folder, deadline, &|| queue.is_stopped(), &unchanged) {
        Ok(()) => {
            deadline.note_removed();
            queue.complete(completion);
        }
        Err(Refused::Stopped(_)) => {}
        Err(Refused::Changed(_)) => queue.fail(changed.take().unwrap_or_default()),
        Err(refused) => {
            let still_open = matches!(refused, Refused::StillOpen(_))
                .then(|| leftovers::still_open(fs(), folder))
                .flatten();
            queue.fail(
                rust_i18n::t!(
                    "Could not delete “%{name}”: %{error}",
                    name = name.to_string_lossy(),
                    error = still_open.map_or_else(
                        || os_error_detail(refused.errno()),
                        crate::services::error_detail
                    )
                )
                .into_owned(),
            );
        }
    }
}

pub(super) fn local_device_is_rotational(fd: impl AsFd) -> Option<bool> {
    fs_ops::assert_off_main_thread();
    let stat = rustix::fs::fstat(fd).ok()?;
    let device = PathBuf::from("/sys/dev/block").join(format!(
        "{}:{}",
        rustix::fs::major(stat.st_dev),
        rustix::fs::minor(stat.st_dev)
    ));
    let device = std::fs::canonicalize(device).ok()?;
    rotational_sysfs_node(&device, &mut HashSet::new())
}

pub(super) fn rotational_sysfs_node(path: &Path, visited: &mut HashSet<PathBuf>) -> Option<bool> {
    let path = std::fs::canonicalize(path).ok()?;
    if !visited.insert(path.clone()) {
        return None;
    }

    let mut has_slave = false;
    let mut unknown_slave = false;
    if let Ok(slaves) = std::fs::read_dir(path.join("slaves")) {
        for slave in slaves.flatten() {
            has_slave = true;
            match rotational_sysfs_node(&slave.path(), visited) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => unknown_slave = true,
            }
        }
    }
    if has_slave {
        return (!unknown_slave).then_some(false);
    }

    let mut current = Some(path.as_path());
    while let Some(node) = current {
        if let Ok(value) = std::fs::read_to_string(node.join("queue/rotational")) {
            return match value.trim() {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            };
        }
        current = node.parent();
    }
    None
}

pub(super) fn bounded_local_worker_count(available: usize, rotational: bool) -> usize {
    let available = available.max(1);
    if rotational {
        1
    } else {
        available.min(MAX_LOCAL_IO_WORKERS)
    }
}

pub(super) fn local_delete_worker_count(roots: &[PinnedEntry]) -> usize {
    let available = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let rotational = roots
        .iter()
        .any(|root| fs().rotational(root.parent.as_fd()) == Some(true));
    bounded_local_worker_count(available, rotational)
}

pub(super) fn parallel_delete_local_blocking_with_workers(
    roots: Vec<PinnedEntry>,
    cancelled: Arc<AtomicBool>,
    worker_count: usize,
    deadline: &Deadline,
) -> Result<(), String> {
    let worker_count = worker_count.max(1);
    let queue = Arc::new(LocalDeleteQueue::new(cancelled.clone()));
    for root in roots {
        queue.enqueue(LocalDeleteJob::Entry {
            parent: root.parent,
            name: root.name,
            expected: root.identity,
            guard: None,
            completion: None,
            deadline: deadline.clone(),
        });
    }

    run_local_delete_workers(&queue, worker_count, |work| {
        thread::Builder::new().spawn(work)
    });

    queue.result()
}

pub(super) fn run_local_delete_workers(
    queue: &Arc<LocalDeleteQueue>,
    worker_count: usize,
    spawn: impl FnMut(Box<dyn FnOnce() + Send>) -> std::io::Result<thread::JoinHandle<()>>,
) {
    work_queue::run_workers(queue, worker_count, Some(10), spawn);
}

pub(super) fn parallel_delete_local(
    roots: Vec<PinnedEntry>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    parallel_delete_local_with_worker_count(roots, cancellable, None, Deadline::default())
}

pub(super) fn parallel_delete_local_with_worker_count(
    roots: Vec<PinnedEntry>,
    cancellable: gio::Cancellable,
    worker_count: Option<usize>,
    deadline: Deadline,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    parallel_delete_local_with(roots, cancellable, move |roots, cancelled| {
        let worker_count = worker_count.unwrap_or_else(|| local_delete_worker_count(&roots));
        parallel_delete_local_blocking_with_workers(roots, cancelled, worker_count, &deadline)
    })
}

pub(super) fn parallel_delete_local_with(
    roots: Vec<PinnedEntry>,
    cancellable: gio::Cancellable,
    delete: impl FnOnce(Vec<PinnedEntry>, Arc<AtomicBool>) -> Result<(), String> + Send + 'static,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if roots.is_empty() {
            return Ok(());
        }
        if cancellable.is_cancelled() {
            return Err(cancelled_local_delete());
        }
        let result = run_cancellable(&cancellable, move |cancelled| delete(roots, cancelled)).await;
        match result.map_err(|_| translated_io_error("The deletion stopped unexpectedly"))? {
            Ok(()) => Ok(()),
            Err(error) if error == "Delete cancelled" => Err(cancelled_local_delete()),
            Err(error) => Err(io_error(error)),
        }
    })
}

/// Traverse pinned directories so a renamed component cannot redirect deletion.
#[cfg(test)]
pub(super) fn permanently_delete_local(
    parent: OwnedFd,
    name: OsString,
    expected: Option<Recorded>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    parallel_delete_local(
        vec![PinnedEntry {
            parent: Arc::new(parent),
            name,
            identity: expected,
        }],
        cancellable,
    )
}

pub(super) fn permanently_delete_local_path_if_unchanged(
    path: PathBuf,
    expected: Option<Recorded>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    permanently_delete_local_path_if_unchanged_with_worker_count(
        path,
        expected,
        cancellable,
        None,
        Deadline::default(),
    )
}

pub(super) fn permanently_delete_local_path_if_unchanged_with_worker_count(
    path: PathBuf,
    expected: Option<Recorded>,
    cancellable: gio::Cancellable,
    worker_count: Option<usize>,
    deadline: Deadline,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let Some(parent_path) = path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error(
                "Cannot permanently delete the filesystem root",
            ));
        };
        let Some(name) = path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid delete target"));
        };
        let parent =
            run_local_delete_step(move || open_local_parent_directory(&parent_path)).await?;
        parallel_delete_local_with_worker_count(
            vec![PinnedEntry {
                parent: Arc::new(parent),
                name,
                identity: expected,
            }],
            cancellable,
            worker_count,
            deadline,
        )
        .await
    })
}

pub(super) async fn local_file_identity(file: &gio::File) -> Result<Option<Recorded>, glib::Error> {
    if !file.is_native() {
        return Ok(None);
    }
    let Some(path) = file.path() else {
        return Ok(None);
    };
    run_local_delete_step(move || {
        let parent_path = path
            .parent()
            .ok_or_else(|| crate::i18n::tr("Cannot inspect the filesystem root"))?;
        let name = path
            .file_name()
            .ok_or_else(|| crate::i18n::tr("Invalid local filesystem target"))?;
        let parent = open_local_parent_directory(parent_path)?;
        let stat = fs()
            .stat(Entry::new(&parent, name), false)
            .map_err(|error| {
                rust_i18n::t!(
                    "Could not inspect %{path}: %{error}",
                    path = path.display(),
                    error = os_error_detail(error)
                )
                .into_owned()
            })?;
        Ok(Recorded::of(&stat))
    })
    .await
    .map(Some)
}

pub(super) fn permanently_delete_maybe_local(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    permanently_delete_maybe_local_if_unchanged(file, directory, None, cancellable)
}

pub(super) fn permanently_delete_maybe_local_with_worker_count(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
    worker_count: Option<usize>,
    deadline: Deadline,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    if file.is_native()
        && let Some(path) = file.path()
    {
        return permanently_delete_local_path_if_unchanged_with_worker_count(
            path,
            None,
            cancellable,
            worker_count,
            deadline,
        );
    }
    permanently_delete(file, directory, cancellable)
}

pub(super) fn permanently_delete_maybe_local_if_unchanged(
    file: gio::File,
    directory: bool,
    expected: Option<Recorded>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    if file.is_native()
        && let Some(path) = file.path()
    {
        return permanently_delete_local_path_if_unchanged(path, expected, cancellable);
    }
    permanently_delete(file, directory, cancellable)
}

#[derive(Clone)]
pub(super) struct DeletionTarget {
    pub(super) location: Location,
    pub(super) display_name: String,
    pub(super) is_directory: bool,
}

impl DeletionTarget {
    pub(super) fn from_entry(entry: &FileEntry) -> Self {
        Self {
            location: entry.location.clone(),
            display_name: entry.display_name.clone(),
            is_directory: entry.is_directory(),
        }
    }

    pub(super) async fn probed(location: &Location, cancellable: &gio::Cancellable) -> Self {
        let file = gio_file_for_location(location);
        let is_directory = await_cancellable(&file, cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await
        .is_ok_and(|info| info.file_type() == gio::FileType::Directory);
        Self {
            location: location.clone(),
            display_name: location.display_name(),
            is_directory,
        }
    }
}

pub(super) async fn local_parent_worker_count(parents: impl IntoIterator<Item = PathBuf>) -> usize {
    let parents = parents.into_iter().collect::<HashSet<_>>();
    gio::spawn_blocking(move || {
        let available = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let rotational = parents.iter().any(|parent| {
            open_local_parent_directory(parent)
                .ok()
                .and_then(|parent| fs().rotational(parent.as_fd()))
                == Some(true)
        });
        bounded_local_worker_count(available, rotational)
    })
    .await
    .unwrap_or(1)
}

pub(super) async fn deletion_worker_count(targets: &[DeletionTarget]) -> usize {
    local_parent_worker_count(targets.iter().filter_map(|target| {
        target
            .location
            .native_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }))
    .await
}

const REMOVED_REPORT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Reports the entries removed so far, since one large folder completes only when its
/// last entry is gone.
pub(super) struct RemovedReporter(glib::JoinHandle<()>);

impl RemovedReporter {
    pub(super) fn start(
        request_id: OperationRequestId,
        deadline: Deadline,
        emit: Rc<dyn Fn(OperationEvent)>,
        interval: std::time::Duration,
    ) -> Self {
        Self(glib::MainContext::default().spawn_local(async move {
            let mut reported = 0;
            loop {
                glib::timeout_future(interval).await;
                let removed = deadline.removed();
                if removed != reported {
                    reported = removed;
                    emit(OperationEvent::DeleteEntriesRemoved {
                        request_id,
                        removed,
                    });
                }
            }
        }))
    }
}

impl Drop for RemovedReporter {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn run_deletion(
    request_id: OperationRequestId,
    targets: Vec<DeletionTarget>,
    permanent: bool,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
) {
    run_deletion_in(
        Rc::new(SystemTrash),
        request_id,
        targets,
        permanent,
        emit,
        cancellable,
    )
    .await;
}

pub(super) async fn run_deletion_in(
    environment: Rc<dyn TrashEnvironment>,
    request_id: OperationRequestId,
    targets: Vec<DeletionTarget>,
    permanent: bool,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
) {
    let mut leftovers = if permanent {
        None
    } else {
        let paths = targets
            .iter()
            .filter_map(|target| target.location.native_path().map(Path::to_owned));
        Some(LeftoverWatch::observe(environment.as_ref(), paths).await)
    };
    let mut deleted_pins = crate::adapters::bookmarks::DeletedPins::default();
    let mut errors = Vec::new();
    let mut deleted_locations = Vec::new();
    let mut failed_locations = Vec::new();
    let mut retryable_locations = Vec::new();
    let mut affected_locations = HashSet::new();
    for target in &targets {
        if let Some(parent) = target.location.parent() {
            affected_locations.insert(parent);
        }
        if target.is_directory {
            affected_locations.insert(target.location.clone());
        }
    }
    if !permanent {
        affected_locations.insert(Location::uri("trash:///"));
    }
    let total = targets.len();
    let mut last_progress_emit = std::time::Instant::now();
    let batch_size = if total > 32 {
        deletion_worker_count(&targets).await
    } else {
        1
    };
    let local_tree_worker_count = (batch_size > 1).then_some(1);
    let mut completed_count = 0;
    let mut pending_progress_locations = Vec::new();
    let mut cut_off = Vec::new();
    let deadline = Deadline::default();
    let _reporter = permanent.then(|| {
        RemovedReporter::start(
            request_id,
            deadline.clone(),
            emit.clone(),
            REMOVED_REPORT_INTERVAL,
        )
    });

    for chunk in targets.chunks(batch_size) {
        if cancellable.is_cancelled() {
            if let Some(leftovers) = leftovers.take() {
                leftovers.clean_up().await;
            }
            emit(cancelled_event(
                request_id,
                deleted_locations,
                failed_locations,
                targets[completed_count..]
                    .iter()
                    .map(|target| target.location.clone())
                    .collect(),
                affected_locations,
            ));
            return;
        }

        let context = glib::MainContext::default();
        let tasks = chunk
            .iter()
            .map(|target| {
                let target = target.clone();
                let operation_target = target.clone();
                let cancellable = cancellable.clone();
                let environment = environment.clone();
                let deadline = deadline.clone();
                let task = context.spawn_local(async move {
                    let file = gio_file_for_location(&operation_target.location);
                    if !permanent {
                        return crate::adapters::trash_support::attempt(
                            environment.as_ref(),
                            &file,
                            &cancellable,
                            &deadline,
                        )
                        .await;
                    }
                    if operation_target
                        .location
                        .uri_value()
                        .is_some_and(|uri| uri.starts_with("trash:"))
                    {
                        await_cancellable(&file, &cancellable, |file, cancellable, result| {
                            file.delete_async(
                                glib::Priority::DEFAULT,
                                Some(cancellable),
                                move |output| result.resolve(output),
                            );
                        })
                        .await
                    } else {
                        permanently_delete_maybe_local_with_worker_count(
                            file,
                            operation_target.is_directory,
                            cancellable.clone(),
                            local_tree_worker_count,
                            deadline,
                        )
                        .await
                    }
                    .map(|()| None)
                    .map_err(TrashError::permanent)
                });
                (target, task)
            })
            .collect();

        let mut chunk_cancelled = false;
        for (target, result) in join_local_tasks(tasks).await {
            completed_count += 1;
            let result = result.unwrap_or_else(|_| {
                Err(TrashError::permanent(translated_io_error(
                    "The deletion stopped unexpectedly",
                )))
            });
            if let (Some(leftovers), Some(path)) =
                (leftovers.as_mut(), target.location.native_path())
            {
                leftovers.note(path, result.as_ref().err().map(|error| error.failure));
            }
            match result {
                Err(error) if error.failure == TrashFailure::Cancelled => {
                    chunk_cancelled = true;
                    if deletion_undone_when_cancelled(&target.location, permanent) {
                        cut_off.push(target.location.clone());
                    } else {
                        failed_locations.push(target.location.clone());
                    }
                }
                Err(error) => {
                    if is_trash_unsupported_failure(permanent, &error) {
                        retryable_locations.push(target.location.clone());
                    }
                    errors.push(deletion_error_message(&target.display_name, &error));
                    failed_locations.push(target.location.clone());
                }
                Ok(identity) => {
                    deleted_pins.push(target.location.clone());
                    if let Some(identity) = identity {
                        emit(OperationEvent::ItemTrashed {
                            request_id,
                            location: target.location.clone(),
                            identity,
                        });
                    }
                    deleted_locations.push(target.location.clone());
                    pending_progress_locations.push(target.location.clone());
                }
            }
            let is_last = completed_count == total;
            let now = std::time::Instant::now();
            if total <= 32
                || is_last
                || now.duration_since(last_progress_emit) >= std::time::Duration::from_millis(33)
            {
                last_progress_emit = now;
                emit(OperationEvent::DeleteProgress {
                    request_id,
                    completed: completed_count,
                    total,
                    deleted_locations: std::mem::take(&mut pending_progress_locations),
                });
            }
        }
        if chunk_cancelled {
            cut_off.extend(
                targets[completed_count..]
                    .iter()
                    .map(|target| target.location.clone()),
            );
            if let Some(leftovers) = leftovers.take() {
                leftovers.clean_up().await;
            }
            emit(cancelled_event(
                request_id,
                deleted_locations,
                failed_locations,
                cut_off,
                affected_locations,
            ));
            return;
        }
    }
    if let Some(leftovers) = leftovers {
        leftovers.clean_up().await;
    }
    if errors.is_empty() {
        emit(OperationEvent::Deleted {
            request_id,
            locations: deleted_locations,
        });
    } else {
        let has_non_retryable_failures = errors.len() > retryable_locations.len();
        emit(OperationEvent::CompletedWithErrors {
            request_id,
            deleted_locations,
            retryable_locations,
            has_non_retryable_failures,
            message: deletion_error_summary(&errors, permanent),
        });
    }
}
