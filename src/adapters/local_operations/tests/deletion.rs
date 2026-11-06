use super::fs_quirks::{Births, HoldGuard, Op, Racer, Trash, hold};
use super::*;
use crate::adapters::local_operations::{
    Deadline, LocalDeleteJob, LocalDeleteQueue, PinnedEntry, RemovedReporter,
    parallel_delete_local_blocking_with_workers, parallel_delete_local_with,
    process_local_delete_job, retry_local_open, run_local_delete_workers,
};
use crate::{
    adapters::trash_support::SystemTrash,
    model::hidden_names::foreign_leftover,
    services::{Holder, file_activity},
};
use std::sync::{Barrier, Mutex, atomic::Ordering, mpsc};

mod open_files;
mod pins;
mod reporting;
mod symlinks;
mod trash;
mod workers;

fn delete_root(path: &Path) -> Result<PinnedEntry, Box<dyn Error>> {
    Ok(PinnedEntry {
        parent: Arc::new(super::super::open_local_parent_directory(
            path.parent().ok_or("no parent")?,
        )?),
        name: path.file_name().ok_or("no name")?.to_owned(),
        identity: None,
    })
}

fn enqueue_root(queue: &Arc<LocalDeleteQueue>, root: PinnedEntry) {
    queue.enqueue(LocalDeleteJob::Entry {
        parent: root.parent,
        name: root.name,
        expected: root.identity,
        guard: None,
        completion: None,
        deadline: Default::default(),
    });
}

fn process_next(queue: &Arc<LocalDeleteQueue>) {
    process_local_delete_job(queue, queue.next_job().expect("queued job"));
    queue.finish_job();
}
