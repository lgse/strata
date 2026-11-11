// SPDX-License-Identifier: MIT

use std::{
    ffi::OsStr,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, atomic::AtomicBool},
    thread,
};
#[cfg(test)]
use std::{ffi::OsString, os::fd::OwnedFd};

use gio::prelude::*;

use super::{
    super::{
        fs_ops::{Entry, fs},
        io_error,
        leftovers::Deadline,
        native_fs::{Recorded, open_local_parent_directory, os_error_detail},
        run_cancellable, translated_io_error, work_queue,
    },
    gio_delete::permanently_delete,
    queue::{LocalDeleteJob, LocalDeleteQueue, PinnedEntry},
    workers::local_delete_worker_count,
};

pub(super) fn cancelled_local_delete() -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Cancelled, "Delete cancelled")
}

pub(in crate::adapters::local_operations) async fn run_local_delete_step<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, glib::Error> {
    gio::spawn_blocking(work)
        .await
        .map_err(|_| translated_io_error("The deletion stopped unexpectedly"))?
        .map_err(io_error)
}

/// How many threads delete a local tree, and the deadline they report removals to.
#[derive(Clone)]
pub(super) struct DeleteWorkers {
    pub(super) count: Option<usize>,
    pub(super) deadline: Deadline,
}

pub(in crate::adapters::local_operations) fn parallel_delete_local_blocking_with_workers(
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

pub(in crate::adapters::local_operations) fn run_local_delete_workers(
    queue: &Arc<LocalDeleteQueue>,
    worker_count: usize,
    spawn: impl FnMut(Box<dyn FnOnce() + Send>) -> std::io::Result<thread::JoinHandle<()>>,
) {
    work_queue::run_workers(queue, worker_count, Some(10), spawn);
}

pub(in crate::adapters::local_operations) fn parallel_delete_local(
    roots: Vec<PinnedEntry>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    parallel_delete_local_with_worker_count(roots, cancellable, None, Deadline::default())
}

pub(in crate::adapters::local_operations) fn parallel_delete_local_with_worker_count(
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

pub(in crate::adapters::local_operations) fn parallel_delete_local_with(
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
pub(in crate::adapters::local_operations) fn permanently_delete_local(
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

pub(in crate::adapters::local_operations) fn permanently_delete_local_path_if_unchanged(
    path: PathBuf,
    expected: Option<Recorded>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    permanently_delete_local_path_if_unchanged_with_workers(
        path,
        expected,
        cancellable,
        DeleteWorkers {
            count: None,
            deadline: Deadline::default(),
        },
    )
}

pub(super) fn permanently_delete_local_path_if_unchanged_with_workers(
    path: PathBuf,
    expected: Option<Recorded>,
    cancellable: gio::Cancellable,
    workers: DeleteWorkers,
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
            workers.count,
            workers.deadline,
        )
        .await
    })
}

pub(in crate::adapters::local_operations) async fn local_file_identity(
    file: &gio::File,
) -> Result<Option<Recorded>, glib::Error> {
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

pub(in crate::adapters::local_operations) fn permanently_delete_maybe_local(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    permanently_delete_maybe_local_if_unchanged(file, directory, None, cancellable)
}

pub(super) fn permanently_delete_maybe_local_with_workers(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
    workers: DeleteWorkers,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    if file.is_native()
        && let Some(path) = file.path()
    {
        return permanently_delete_local_path_if_unchanged_with_workers(
            path,
            None,
            cancellable,
            workers,
        );
    }
    permanently_delete(file, directory, cancellable)
}

pub(in crate::adapters::local_operations) fn permanently_delete_maybe_local_if_unchanged(
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
