// SPDX-License-Identifier: MIT

use super::copy_engine::SharedProgress;
use crate::{
    model::Location,
    services::{OperationEvent, OperationRequestId, TransferReport},
};
use gtk::{gio, glib, prelude::*};

use super::{await_cancellable, translated_io_error};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub(super) const TRANSFER_PROGRESS_INTERVAL: Duration = Duration::from_millis(33);
// Smaller copies normally finish before the first sample; keep them on GIO's lower-overhead path.
pub(super) const BYTE_PROGRESS_MIN_FILE_SIZE: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Reported {
    pub(super) bytes: u64,
    pub(super) files: usize,
}

impl Reported {
    pub(super) fn since(self, earlier: Self) -> Self {
        Self {
            bytes: self.bytes.saturating_sub(earlier.bytes),
            files: self.files.saturating_sub(earlier.files),
        }
    }
}

pub(super) struct TransferProgressTracker {
    pub(super) request_id: OperationRequestId,
    pub(super) completed_items: Cell<usize>,
    pub(super) completed_files: Cell<usize>,
    pub(super) total_files: Option<usize>,
    pub(super) current_file: RefCell<Option<String>>,
    pub(super) transferred_bytes: Cell<u64>,
    pub(super) total_bytes: Option<u64>,
    /// What finished items reported beyond their estimates, as files that grew.
    pub(super) excess: Cell<Reported>,
    pub(super) last_emit: Cell<Option<Instant>>,
    pub(super) emit: Rc<dyn Fn(OperationEvent)>,
}

impl TransferProgressTracker {
    pub(super) fn new(
        request_id: OperationRequestId,
        total_bytes: Option<u64>,
        total_files: Option<usize>,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> Rc<Self> {
        Rc::new(Self {
            request_id,
            completed_items: Cell::new(0),
            completed_files: Cell::new(0),
            total_files,
            current_file: RefCell::new(None),
            transferred_bytes: Cell::new(0),
            total_bytes,
            excess: Cell::new(Reported::default()),
            last_emit: Cell::new(None),
            emit,
        })
    }

    pub(super) fn totals(&self) -> (Option<u64>, Option<usize>) {
        let excess = self.excess.get();
        (
            self.total_bytes.map(|total| {
                total
                    .saturating_add(excess.bytes)
                    .max(self.transferred_bytes.get())
            }),
            self.total_files.map(|total| {
                total
                    .saturating_add(excess.files)
                    .max(self.completed_files.get())
            }),
        )
    }

    pub(super) fn reported(&self) -> Reported {
        Reported {
            bytes: self.transferred_bytes.get(),
            files: self.completed_files.get(),
        }
    }

    pub(super) fn emit(&self) {
        self.last_emit.set(Some(Instant::now()));
        self.emit_progress(None);
    }

    pub(super) fn emit_if_due(&self) {
        if self
            .last_emit
            .get()
            .is_none_or(|last| last.elapsed() >= TRANSFER_PROGRESS_INTERVAL)
        {
            self.emit();
        }
    }

    pub(super) fn emit_progress(&self, created_location: Option<Location>) {
        let (total_bytes, total_files) = self.totals();
        (self.emit)(OperationEvent::TransferProgress {
            request_id: self.request_id,
            completed_items: self.completed_items.get(),
            completed_files: self.completed_files.get(),
            total_files,
            current_file: self.current_file.borrow().clone(),
            transferred_bytes: self.transferred_bytes.get(),
            total_bytes,
            created_location,
        });
    }

    /// Emits at once when the last file completes, so it is never throttled away.
    pub(super) fn emit_after_files(&self) {
        if self.totals().1 == Some(self.completed_files.get()) {
            self.emit();
        } else {
            self.emit_if_due();
        }
    }

    pub(super) fn report(&self, report: TransferReport) {
        if !report.is_empty() {
            (self.emit)(OperationEvent::ItemsReported {
                request_id: self.request_id,
                report,
            });
        }
    }

    pub(super) fn set_current_file(&self, name: String) {
        self.current_file.replace(Some(name));
        self.emit_if_due();
    }

    pub(super) fn follow(self: &Rc<Self>, shared: &Arc<SharedProgress>) -> ProgressFollower {
        let seen = Rc::new(Cell::new(Reported::default()));
        let (tracker, timer_shared, timer_seen) = (self.clone(), shared.clone(), seen.clone());
        let source = glib::timeout_add_local(TRANSFER_PROGRESS_INTERVAL, move || {
            tracker.fold(&timer_shared, &timer_seen);
            glib::ControlFlow::Continue
        });
        ProgressFollower {
            tracker: self.clone(),
            shared: shared.clone(),
            seen,
            source: Some(source),
        }
    }

    pub(super) fn fold(&self, shared: &SharedProgress, seen: &Cell<Reported>) {
        if let Some(name) = shared.take_current() {
            self.current_file.replace(Some(name));
        }
        let now = shared.reported();
        let delta = now.since(seen.replace(now));
        if delta == Reported::default() {
            return;
        }
        self.transferred_bytes
            .set(self.transferred_bytes.get().saturating_add(delta.bytes));
        self.completed_files
            .set(self.completed_files.get().saturating_add(delta.files));
        if delta.files > 0 {
            self.emit_after_files();
        } else {
            self.emit_if_due();
        }
    }

    pub(super) fn add_bytes(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.transferred_bytes
            .set(self.transferred_bytes.get().saturating_add(bytes));
        self.emit_if_due();
    }

    pub(super) fn begin_file(
        self: &Rc<Self>,
        name: String,
        track_bytes: bool,
    ) -> FileTransferProgress {
        self.set_current_file(name);
        let byte_progress = track_bytes.then(|| {
            let reported_bytes = Arc::new(AtomicU64::new(0));
            let reported_total = Arc::new(AtomicU64::new(0));
            let observed_bytes = Rc::new(Cell::new(0));
            let tracker = self.clone();
            let timer_reported = reported_bytes.clone();
            let timer_observed = observed_bytes.clone();
            let update_source = glib::timeout_add_local(TRANSFER_PROGRESS_INTERVAL, move || {
                record_file_progress(&tracker, &timer_reported, &timer_observed);
                glib::ControlFlow::Continue
            });
            FileByteProgress {
                reported_bytes,
                reported_total,
                observed_bytes,
                update_source: RefCell::new(Some(update_source)),
            }
        });
        FileTransferProgress {
            tracker: self.clone(),
            byte_progress,
        }
    }

    /// Credits what the item owes beyond what it `reported`, so concurrent
    /// items are never counted twice.
    pub(super) fn finish_item(
        &self,
        reported: Reported,
        expected_bytes: Option<u64>,
        expected_files: Option<usize>,
        created_location: Option<Location>,
    ) {
        let mut excess = self.excess.get();
        if let Some(expected_bytes) = expected_bytes {
            self.transferred_bytes.set(
                self.transferred_bytes
                    .get()
                    .saturating_add(expected_bytes.saturating_sub(reported.bytes)),
            );
            excess.bytes = excess
                .bytes
                .saturating_add(reported.bytes.saturating_sub(expected_bytes));
        }
        if let Some(expected_files) = expected_files {
            self.completed_files.set(
                self.completed_files
                    .get()
                    .saturating_add(expected_files.saturating_sub(reported.files)),
            );
            excess.files = excess
                .files
                .saturating_add(reported.files.saturating_sub(expected_files));
        }
        self.excess.set(excess);
        self.completed_items
            .set(self.completed_items.get().saturating_add(1));
        self.last_emit.set(Some(Instant::now()));
        self.emit_progress(created_location);
    }
}

pub(super) struct ProgressFollower {
    pub(super) tracker: Rc<TransferProgressTracker>,
    pub(super) shared: Arc<SharedProgress>,
    pub(super) seen: Rc<Cell<Reported>>,
    pub(super) source: Option<glib::SourceId>,
}

impl ProgressFollower {
    /// Stops following and folds in the last report, so a late tick cannot
    /// count it twice.
    pub(super) fn stop(mut self) -> Reported {
        if let Some(source) = self.source.take() {
            source.remove();
        }
        self.tracker.fold(&self.shared, &self.seen);
        self.seen.get()
    }
}

impl Drop for ProgressFollower {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            source.remove();
        }
    }
}

pub(super) struct FileTransferProgress {
    pub(super) tracker: Rc<TransferProgressTracker>,
    pub(super) byte_progress: Option<FileByteProgress>,
}

pub(super) struct FileByteProgress {
    pub(super) reported_bytes: Arc<AtomicU64>,
    pub(super) reported_total: Arc<AtomicU64>,
    pub(super) observed_bytes: Rc<Cell<u64>>,
    pub(super) update_source: RefCell<Option<glib::SourceId>>,
}

impl FileTransferProgress {
    pub(super) fn reporter(&self) -> Option<FileProgressReporter> {
        self.byte_progress
            .as_ref()
            .map(|progress| FileProgressReporter {
                reported_bytes: progress.reported_bytes.clone(),
                reported_total: progress.reported_total.clone(),
            })
    }

    pub(super) fn finish(&self, bytes: Option<u64>) {
        if let Some(progress) = &self.byte_progress {
            progress.stop_updates();
            let final_bytes = bytes
                .unwrap_or_else(|| progress.reported_total.load(Ordering::Relaxed))
                .max(progress.reported_bytes.load(Ordering::Relaxed));
            progress
                .reported_bytes
                .fetch_max(final_bytes, Ordering::Relaxed);
            record_file_progress(
                &self.tracker,
                &progress.reported_bytes,
                &progress.observed_bytes,
            );
        } else if let Some(bytes) = bytes {
            self.tracker
                .transferred_bytes
                .set(self.tracker.transferred_bytes.get().saturating_add(bytes));
        }
        self.tracker
            .completed_files
            .set(self.tracker.completed_files.get().saturating_add(1));
        self.tracker.emit_after_files();
    }
}

impl Drop for FileTransferProgress {
    fn drop(&mut self) {
        if let Some(progress) = &self.byte_progress {
            progress.stop_updates();
        }
    }
}

impl FileByteProgress {
    pub(super) fn stop_updates(&self) {
        if let Some(source) = self.update_source.borrow_mut().take() {
            source.remove();
        }
    }
}

#[derive(Clone)]
pub(super) struct FileProgressReporter {
    pub(super) reported_bytes: Arc<AtomicU64>,
    pub(super) reported_total: Arc<AtomicU64>,
}

impl FileProgressReporter {
    pub(super) fn record(&self, current: i64, total: i64) {
        self.reported_bytes
            .fetch_max(current.max(0) as u64, Ordering::Relaxed);
        if total >= 0 {
            self.reported_total
                .fetch_max(total as u64, Ordering::Relaxed);
        }
    }
}

pub(super) fn record_file_progress(
    tracker: &TransferProgressTracker,
    reported_bytes: &AtomicU64,
    observed_bytes: &Cell<u64>,
) {
    let current = reported_bytes.load(Ordering::Relaxed);
    let previous = observed_bytes.replace(current);
    tracker.add_bytes(current.saturating_sub(previous));
}

pub(super) async fn run_file_operation_with_progress(
    reporter: FileProgressReporter,
    operation: impl FnOnce(&mut dyn FnMut(i64, i64)) -> Result<(), glib::Error> + Send + 'static,
    worker_error: &'static str,
) -> Result<(), glib::Error> {
    gio::spawn_blocking(move || {
        let mut callback = move |current, total| reporter.record(current, total);
        operation(&mut callback)
    })
    .await
    .map_err(|_| translated_io_error(worker_error))?
}

pub(super) async fn copy_file_with_progress(
    source: gio::File,
    target: gio::File,
    flags: gio::FileCopyFlags,
    cancellable: gio::Cancellable,
    reporter: Option<FileProgressReporter>,
) -> Result<(), glib::Error> {
    let Some(reporter) = reporter else {
        return await_cancellable(&source, &cancellable, move |source, cancellable, result| {
            source.copy_async(
                &target,
                flags,
                glib::Priority::DEFAULT,
                Some(cancellable),
                None,
                move |output| result.resolve(output),
            );
        })
        .await;
    };
    run_file_operation_with_progress(
        reporter,
        move |progress| source.copy(&target, flags, Some(&cancellable), Some(progress)),
        "The copy stopped unexpectedly",
    )
    .await
}

pub(super) async fn move_file_with_progress(
    source: gio::File,
    target: gio::File,
    flags: gio::FileCopyFlags,
    cancellable: gio::Cancellable,
    reporter: Option<FileProgressReporter>,
) -> Result<(), glib::Error> {
    let Some(reporter) = reporter else {
        return await_cancellable(&source, &cancellable, move |source, cancellable, result| {
            source.move_async(
                &target,
                flags,
                glib::Priority::DEFAULT,
                Some(cancellable),
                None,
                move |output| result.resolve(output),
            );
        })
        .await;
    };
    run_file_operation_with_progress(
        reporter,
        move |progress| source.move_(&target, flags, Some(&cancellable), Some(progress)),
        "The move stopped unexpectedly",
    )
    .await
}
