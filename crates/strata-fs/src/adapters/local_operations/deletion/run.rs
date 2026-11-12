// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use gio::prelude::*;

use super::{
    super::{
        await_cancellable, cancelled_event, deletion_error_message, deletion_error_summary,
        deletion_undone_when_cancelled, is_trash_unsupported_failure, join_local_tasks,
        leftovers::Deadline, translated_io_error,
    },
    parallel::{DeleteWorkers, permanently_delete_maybe_local_with_workers},
    workers::deletion_worker_count,
};
use crate::{
    adapters::{
        bookmarks::DeletedPins,
        gio_file_for_location,
        trash_support::{
            self, LeftoverWatch, SystemTrash, TrashEnvironment, TrashError, TrashFailure,
        },
    },
    model::{FileEntry, Location},
    services::{ItemIdentity, OperationEvent, OperationRequestId},
};

#[derive(Clone)]
pub(in crate::adapters::local_operations) struct DeletionTarget {
    pub(in crate::adapters::local_operations) location: Location,
    pub(in crate::adapters::local_operations) display_name: String,
    pub(in crate::adapters::local_operations) is_directory: bool,
}

impl DeletionTarget {
    pub(in crate::adapters::local_operations) fn from_entry(entry: &FileEntry) -> Self {
        Self {
            location: entry.location.clone(),
            display_name: entry.display_name.clone(),
            is_directory: entry.is_directory(),
        }
    }

    pub(in crate::adapters::local_operations) async fn probed(
        location: &Location,
        cancellable: &gio::Cancellable,
    ) -> Self {
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

const REMOVED_REPORT_INTERVAL: Duration = Duration::from_millis(250);

/// Above this many targets they are deleted in parallel batches and progress is throttled.
const BATCHED_TARGETS: usize = 32;

const PROGRESS_INTERVAL: Duration = Duration::from_millis(33);

/// Reports the entries removed so far, since one large folder completes only when its
/// last entry is gone.
pub(in crate::adapters::local_operations) struct RemovedReporter(glib::JoinHandle<()>);

impl RemovedReporter {
    pub(in crate::adapters::local_operations) fn start(
        request_id: OperationRequestId,
        deadline: Deadline,
        emit: Rc<dyn Fn(OperationEvent)>,
        interval: Duration,
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

pub(in crate::adapters::local_operations) async fn run_deletion(
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

pub(in crate::adapters::local_operations) async fn run_deletion_in(
    environment: Rc<dyn TrashEnvironment>,
    request_id: OperationRequestId,
    targets: Vec<DeletionTarget>,
    permanent: bool,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
) {
    let leftovers = watch_leftovers(environment.as_ref(), &targets, permanent).await;
    let affected_locations = affected_locations(&targets, permanent);
    let total = targets.len();
    let started = Instant::now();
    let batch_size = if total > BATCHED_TARGETS {
        deletion_worker_count(&targets).await
    } else {
        1
    };
    let deadline = Deadline::default();
    let reporter = permanent.then(|| {
        RemovedReporter::start(
            request_id,
            deadline.clone(),
            emit.clone(),
            REMOVED_REPORT_INTERVAL,
        )
    });
    let deletion = Deletion {
        _reporter: reporter,
        request_id,
        batch_size,
        progress: Progress {
            total,
            completed: 0,
            last_emit: started,
            pending_locations: Vec::new(),
        },
        outcome: Outcome {
            affected_locations,
            ..Outcome::default()
        },
        deleted_pins: DeletedPins::default(),
        leftovers,
        deleter: Deleter {
            environment,
            permanent,
            // Trees inside a parallel batch are walked by one worker each.
            local_tree_worker_count: (batch_size > 1).then_some(1),
            cancellable,
            deadline,
        },
        emit,
    };
    deletion.run(&targets).await;
}

async fn watch_leftovers(
    environment: &dyn TrashEnvironment,
    targets: &[DeletionTarget],
    permanent: bool,
) -> Option<LeftoverWatch> {
    if permanent {
        return None;
    }
    let paths = targets
        .iter()
        .filter_map(|target| target.location.native_path().map(Path::to_owned));
    Some(LeftoverWatch::observe(environment, paths).await)
}

fn affected_locations(targets: &[DeletionTarget], permanent: bool) -> HashSet<Location> {
    let mut affected_locations = HashSet::new();
    for target in targets {
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
    affected_locations
}

type DeleteResult = Result<Option<ItemIdentity>, TrashError>;

/// Deletes one target the way the request asked for.
#[derive(Clone)]
struct Deleter {
    environment: Rc<dyn TrashEnvironment>,
    permanent: bool,
    local_tree_worker_count: Option<usize>,
    cancellable: gio::Cancellable,
    deadline: Deadline,
}

impl Deleter {
    fn spawn_chunk(
        &self,
        chunk: &[DeletionTarget],
    ) -> Vec<(DeletionTarget, glib::JoinHandle<DeleteResult>)> {
        let context = glib::MainContext::default();
        chunk
            .iter()
            .map(|target| {
                let target = target.clone();
                let task = context.spawn_local(self.clone().delete(target.clone()));
                (target, task)
            })
            .collect()
    }

    async fn delete(self, target: DeletionTarget) -> DeleteResult {
        let file = gio_file_for_location(&target.location);
        if !self.permanent {
            return trash_support::attempt(
                self.environment.as_ref(),
                &file,
                &self.cancellable,
                &self.deadline,
            )
            .await;
        }
        if is_trash_item(&target.location) {
            delete_trash_item(&file, &self.cancellable).await
        } else {
            permanently_delete_maybe_local_with_workers(
                file,
                target.is_directory,
                self.cancellable.clone(),
                DeleteWorkers {
                    count: self.local_tree_worker_count,
                    deadline: self.deadline,
                },
            )
            .await
        }
        .map(|()| None)
        .map_err(TrashError::permanent)
    }
}

fn is_trash_item(location: &Location) -> bool {
    location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
}

async fn delete_trash_item(
    file: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    await_cancellable(file, cancellable, |file, cancellable, result| {
        file.delete_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
            result.resolve(output)
        });
    })
    .await
}

struct Progress {
    total: usize,
    completed: usize,
    last_emit: Instant,
    pending_locations: Vec<Location>,
}

impl Progress {
    fn due(&self, now: Instant) -> bool {
        let is_last = self.completed == self.total;
        self.total <= BATCHED_TARGETS
            || is_last
            || now.duration_since(self.last_emit) >= PROGRESS_INTERVAL
    }
}

#[derive(Default)]
struct Outcome {
    errors: Vec<String>,
    deleted_locations: Vec<Location>,
    failed_locations: Vec<Location>,
    retryable_locations: Vec<Location>,
    cut_off: Vec<Location>,
    affected_locations: HashSet<Location>,
}

/// Field order matters: the reporter stops before the pins are forgotten, and the
/// environment outlives its leftover watch.
struct Deletion {
    _reporter: Option<RemovedReporter>,
    request_id: OperationRequestId,
    batch_size: usize,
    progress: Progress,
    outcome: Outcome,
    deleted_pins: DeletedPins,
    leftovers: Option<LeftoverWatch>,
    deleter: Deleter,
    emit: Rc<dyn Fn(OperationEvent)>,
}

impl Deletion {
    async fn run(mut self, targets: &[DeletionTarget]) {
        for chunk in targets.chunks(self.batch_size) {
            if self.deleter.cancellable.is_cancelled() {
                let not_attempted = self.remaining(targets);
                self.finish_cancelled(not_attempted).await;
                return;
            }
            let tasks = self.deleter.spawn_chunk(chunk);
            if self.collect(tasks).await {
                let remaining = self.remaining(targets);
                self.outcome.cut_off.extend(remaining);
                let cut_off = std::mem::take(&mut self.outcome.cut_off);
                self.finish_cancelled(cut_off).await;
                return;
            }
        }
        self.finish().await;
    }

    fn remaining(&self, targets: &[DeletionTarget]) -> Vec<Location> {
        targets[self.progress.completed..]
            .iter()
            .map(|target| target.location.clone())
            .collect()
    }

    /// Returns whether the chunk was cut short by a cancellation.
    async fn collect(
        &mut self,
        tasks: Vec<(DeletionTarget, glib::JoinHandle<DeleteResult>)>,
    ) -> bool {
        let mut chunk_cancelled = false;
        for (target, result) in join_local_tasks(tasks).await {
            self.progress.completed += 1;
            let result = result.unwrap_or_else(|_| {
                Err(TrashError::permanent(translated_io_error(
                    "The deletion stopped unexpectedly",
                )))
            });
            if let (Some(leftovers), Some(path)) =
                (self.leftovers.as_mut(), target.location.native_path())
            {
                leftovers.note(path, result.as_ref().err().map(|error| error.failure));
            }
            chunk_cancelled |= self.record(&target, result);
            self.emit_progress_if_due();
        }
        chunk_cancelled
    }

    /// Returns whether the target's deletion was cancelled.
    fn record(&mut self, target: &DeletionTarget, result: DeleteResult) -> bool {
        match result {
            Err(error) if error.failure == TrashFailure::Cancelled => {
                self.record_cancelled(target);
                true
            }
            Err(error) => {
                self.record_failure(target, &error);
                false
            }
            Ok(identity) => {
                self.record_deleted(target, identity);
                false
            }
        }
    }

    fn record_cancelled(&mut self, target: &DeletionTarget) {
        if deletion_undone_when_cancelled(&target.location, self.deleter.permanent) {
            self.outcome.cut_off.push(target.location.clone());
        } else {
            self.outcome.failed_locations.push(target.location.clone());
        }
    }

    fn record_failure(&mut self, target: &DeletionTarget, error: &TrashError) {
        if is_trash_unsupported_failure(self.deleter.permanent, error) {
            self.outcome
                .retryable_locations
                .push(target.location.clone());
        }
        self.outcome
            .errors
            .push(deletion_error_message(&target.display_name, error));
        self.outcome.failed_locations.push(target.location.clone());
    }

    fn record_deleted(&mut self, target: &DeletionTarget, identity: Option<ItemIdentity>) {
        self.deleted_pins.push(target.location.clone());
        if let Some(identity) = identity {
            (self.emit)(OperationEvent::ItemTrashed {
                request_id: self.request_id,
                location: target.location.clone(),
                identity,
            });
        }
        self.outcome.deleted_locations.push(target.location.clone());
        self.progress
            .pending_locations
            .push(target.location.clone());
    }

    fn emit_progress_if_due(&mut self) {
        let now = Instant::now();
        if !self.progress.due(now) {
            return;
        }
        self.progress.last_emit = now;
        (self.emit)(OperationEvent::DeleteProgress {
            request_id: self.request_id,
            completed: self.progress.completed,
            total: self.progress.total,
            deleted_locations: std::mem::take(&mut self.progress.pending_locations),
        });
    }

    async fn finish_cancelled(mut self, not_attempted: Vec<Location>) {
        if let Some(leftovers) = self.leftovers.take() {
            leftovers.clean_up().await;
        }
        (self.emit)(cancelled_event(
            self.request_id,
            std::mem::take(&mut self.outcome.deleted_locations),
            std::mem::take(&mut self.outcome.failed_locations),
            not_attempted,
            std::mem::take(&mut self.outcome.affected_locations),
        ));
    }

    async fn finish(mut self) {
        if let Some(leftovers) = self.leftovers.take() {
            leftovers.clean_up().await;
        }
        let outcome = std::mem::take(&mut self.outcome);
        if outcome.errors.is_empty() {
            (self.emit)(OperationEvent::Deleted {
                request_id: self.request_id,
                locations: outcome.deleted_locations,
            });
            return;
        }
        let has_non_retryable_failures = outcome.errors.len() > outcome.retryable_locations.len();
        (self.emit)(OperationEvent::CompletedWithErrors {
            request_id: self.request_id,
            deleted_locations: outcome.deleted_locations,
            retryable_locations: outcome.retryable_locations,
            has_non_retryable_failures,
            message: deletion_error_summary(&outcome.errors, self.deleter.permanent),
        });
    }
}
