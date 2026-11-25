// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    ffi::OsString,
    io,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{
    super::{
        fs_ops::{Entry, fs},
        leftovers::{self, Deadline, Refused},
        native_fs::{Recorded, os_error_detail},
        work_queue::{Group, Job, WorkQueue},
    },
    error::LocalDeleteError,
    pinned::{LocalDeleteStep, ensure_local_delete_target_unchanged, open_local_delete_target},
};

#[derive(Clone)]
pub(in crate::adapters::local_operations) struct PinnedEntry {
    pub(in crate::adapters::local_operations) parent: Arc<OwnedFd>,
    pub(in crate::adapters::local_operations) name: OsString,
    pub(in crate::adapters::local_operations) identity: Option<Recorded>,
}

pub(in crate::adapters::local_operations) struct LocalDeleteGuard {
    parent: Arc<OwnedFd>,
    name: OsString,
    handle: Arc<OwnedFd>,
    relative: Arc<Path>,
}

impl LocalDeleteGuard {
    fn ensure_unchanged(&self) -> Result<(), String> {
        ensure_local_delete_target_unchanged(self.parent.as_ref(), &self.name, self.handle.as_ref())
    }
}

pub(in crate::adapters::local_operations) type LocalDeleteQueue = WorkQueue<LocalDeleteJob>;

pub(in crate::adapters::local_operations) enum LocalDeleteJob {
    Entry {
        parent: Arc<OwnedFd>,
        name: OsString,
        expected: Option<Recorded>,
        guard: Option<Arc<LocalDeleteGuard>>,
        completion: Option<Arc<Group<LocalDeleteJob>>>,
        deadline: Deadline,
    },
    RemoveDirectory(DirectoryRemoval),
}

impl Job for LocalDeleteJob {
    type Error = LocalDeleteError;

    fn run(self, queue: &LocalDeleteQueue) {
        process_local_delete_job(queue, self);
    }

    fn cancelled() -> Self::Error {
        LocalDeleteError::Cancelled
    }

    fn panicked() -> Self::Error {
        "Delete worker panicked".to_owned().into()
    }

    fn not_started(error: io::Error) -> Self::Error {
        format!("Could not start delete worker: {error}").into()
    }
}

pub(in crate::adapters::local_operations) fn process_local_delete_job(
    queue: &LocalDeleteQueue,
    job: LocalDeleteJob,
) {
    match job {
        LocalDeleteJob::Entry {
            parent,
            name,
            expected,
            guard,
            completion,
            deadline,
        } => EntryDeletion {
            parent,
            name,
            expected,
            guard,
            completion,
            deadline,
        }
        .run(queue),
        LocalDeleteJob::RemoveDirectory(removal) => remove_local_delete_directory(queue, removal),
    }
}

struct EntryDeletion {
    parent: Arc<OwnedFd>,
    name: OsString,
    expected: Option<Recorded>,
    guard: Option<Arc<LocalDeleteGuard>>,
    completion: Option<Arc<Group<LocalDeleteJob>>>,
    deadline: Deadline,
}

impl EntryDeletion {
    fn run(self, queue: &LocalDeleteQueue) {
        if queue.is_stopped() {
            return;
        }
        let step = match self.open_target() {
            Ok(step) => step,
            Err(error) => {
                queue.fail(error);
                return;
            }
        };
        match step {
            LocalDeleteStep::Removed => {
                self.deadline.note_removed();
                queue.complete(self.completion);
            }
            LocalDeleteStep::Leftover => queue.complete(self.completion),
            LocalDeleteStep::Directory { handle, children } => {
                self.descend(queue, Arc::new(handle), children);
            }
        }
    }

    fn open_target(&self) -> Result<LocalDeleteStep, LocalDeleteError> {
        let in_deleted_folder = self.guard.is_some();
        if let Some(guard) = &self.guard {
            guard.ensure_unchanged()?;
        }
        open_local_delete_target(
            self.parent.as_ref(),
            &self.name,
            self.expected,
            in_deleted_folder,
        )
        .map_err(|error| error.at(self.child_path()))
    }

    fn child_path(&self) -> Option<PathBuf> {
        Some(self.guard.as_ref()?.relative.join(&self.name))
    }

    fn descend(self, queue: &LocalDeleteQueue, handle: Arc<OwnedFd>, children: Vec<OsString>) {
        let relative = self.child_path().unwrap_or_default();
        let removal = DirectoryRemoval {
            parent: self.parent,
            name: self.name,
            handle,
            relative: Arc::from(relative),
            completion: self.completion,
            deadline: self.deadline,
        };
        if children.is_empty() {
            remove_local_delete_directory(queue, removal);
            return;
        }
        let guard = Arc::new(LocalDeleteGuard {
            parent: removal.parent.clone(),
            name: removal.name.clone(),
            handle: removal.handle.clone(),
            relative: removal.relative.clone(),
        });
        let parent = removal.handle.clone();
        let deadline = removal.deadline.clone();
        let group = Group::new(children.len(), LocalDeleteJob::RemoveDirectory(removal));
        for child in children {
            queue.enqueue(LocalDeleteJob::Entry {
                parent: parent.clone(),
                name: child,
                expected: None,
                guard: Some(guard.clone()),
                completion: Some(group.clone()),
                deadline: deadline.clone(),
            });
        }
    }
}

pub(in crate::adapters::local_operations) struct DirectoryRemoval {
    parent: Arc<OwnedFd>,
    name: OsString,
    handle: Arc<OwnedFd>,
    relative: Arc<Path>,
    completion: Option<Arc<Group<LocalDeleteJob>>>,
    deadline: Deadline,
}

impl DirectoryRemoval {
    fn ensure_unchanged(&self) -> Result<(), String> {
        ensure_local_delete_target_unchanged(self.parent.as_ref(), &self.name, self.handle.as_ref())
    }
}

fn remove_local_delete_directory(queue: &LocalDeleteQueue, removal: DirectoryRemoval) {
    if queue.is_stopped() {
        return;
    }
    if let Err(error) = removal.ensure_unchanged() {
        queue.fail(error.into());
        return;
    }
    let changed = Cell::new(None);
    let unchanged = || {
        removal
            .ensure_unchanged()
            .map_err(|error| changed.set(Some(error)))
            .is_ok()
    };
    let folder = Entry::new(removal.parent.as_ref(), &removal.name);
    let deadline = &removal.deadline;
    match leftovers::remove_folder(fs(), folder, deadline, &|| queue.is_stopped(), &unchanged) {
        Ok(()) => {
            deadline.note_removed();
            queue.complete(removal.completion);
        }
        Err(Refused::Stopped(_)) => {}
        Err(Refused::Changed(_)) => queue.fail(changed.take().unwrap_or_default().into()),
        Err(refused) => refuse_directory(queue, folder, &removal, refused),
    }
}

fn refuse_directory(
    queue: &LocalDeleteQueue,
    folder: Entry<'_>,
    removal: &DirectoryRemoval,
    refused: Refused,
) {
    let open = matches!(refused, Refused::StillOpen(_));
    let still_open = open.then(|| leftovers::still_open(fs(), folder)).flatten();
    let reason = still_open.map_or_else(
        || os_error_detail(refused.errno()),
        crate::services::error_detail,
    );
    let relative =
        (!removal.relative.as_os_str().is_empty()).then(|| removal.relative.to_path_buf());
    let error = LocalDeleteError::refused(&removal.name, reason).at(relative);
    // A folder kept only by open files leaves its parent in place but nothing else, so
    // what the delete removes does not depend on which worker reached it first.
    if open {
        queue.defer(error);
    } else {
        queue.fail(error);
    }
}
