// SPDX-License-Identifier: MIT

mod gio_delete;
mod parallel;
mod pinned;
mod queue;
mod run;
mod workers;

pub(super) use gio_delete::{permanently_delete, permanently_delete_keeping};
pub(super) use parallel::{
    local_file_identity, parallel_delete_local, parallel_delete_local_blocking_with_workers,
    parallel_delete_local_with, parallel_delete_local_with_worker_count,
    permanently_delete_maybe_local, permanently_delete_maybe_local_if_unchanged,
};
#[cfg(test)]
pub(super) use parallel::{
    permanently_delete_local, permanently_delete_local_path_if_unchanged, run_local_delete_step,
    run_local_delete_workers,
};
#[cfg(test)]
pub(super) use pinned::{
    LocalDeleteStep, ensure_local_delete_target_unchanged, open_local_delete_target,
};
pub(super) use queue::PinnedEntry;
#[cfg(test)]
pub(super) use queue::{LocalDeleteJob, LocalDeleteQueue, process_local_delete_job};
pub(super) use run::{DeletionTarget, run_deletion};
#[cfg(test)]
pub(super) use run::{RemovedReporter, run_deletion_in};
pub(super) use workers::{
    bounded_local_worker_count, local_delete_worker_count, local_device_is_rotational,
    local_parent_worker_count,
};
