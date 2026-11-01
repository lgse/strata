// SPDX-License-Identifier: MIT

//! Deletions a transfer owes once its copies are on disk: the sources of
//! moves that had to copy, Replace originals moved out of the way, and the
//! originals a merge set aside. A paste batches them per destination
//! filesystem, so one barrier serves many items.

#[cfg(test)]
use std::cell::Cell;
use std::{
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gtk::{gio, glib};

use super::{
    copy_engine::{self, Barrier, Manifest, Unlinks},
    delete_moved_sources,
    deletion::{permanently_delete_keeping, permanently_delete_maybe_local},
    device_flush_failure, io_error,
    merge_aside::MergeAside,
    progress::TransferProgressTracker,
    removable_roots_for_paths,
    replacement::{DisplacedOriginal, original_not_removed, remove_displaced_original},
};
use crate::{
    adapters::{location_for_file, volume::MountKey},
    model::Location,
    services::{
        KeptCause, KeptItem, OperationEvent, OperationRequestId, PasteRequest, SpecialKind,
        TransferConflict, TransferReport,
    },
};

pub(super) enum Deletion {
    Sources(gio::File, Manifest),
    /// With the links a GIO copy left out of it, which stay.
    Source(gio::File, Vec<gio::File>),
    Displaced(DisplacedOriginal, PathBuf),
    Aside(MergeAside),
}

impl Deletion {
    async fn run(
        self,
        progress: Option<&TransferProgressTracker>,
        unlinks: &Arc<Unlinks>,
        cancellable: &gio::Cancellable,
    ) -> Result<(), glib::Error> {
        match self {
            Self::Sources(source, manifest) => {
                delete_moved_sources(&source, manifest, unlinks.clone(), progress, cancellable)
                    .await
            }
            Self::Source(source, kept) if kept.is_empty() => {
                permanently_delete_maybe_local(source, true, cancellable.clone()).await
            }
            Self::Source(source, kept) => {
                if let Some(progress) = progress {
                    progress.report(TransferReport {
                        kept: kept
                            .iter()
                            .filter_map(|link| {
                                Some(KeptItem {
                                    location: location_for_file(link)?,
                                    cause: KeptCause::Skipped(SpecialKind::Symlink),
                                })
                            })
                            .collect(),
                        ..TransferReport::default()
                    });
                }
                permanently_delete_keeping(source, Rc::new(kept), cancellable.clone()).await
            }
            Self::Displaced(original, target) => {
                let kept_in = original.kept_in().to_os_string();
                let kept = target.with_file_name(&kept_in).join(&original.name);
                let Err(error) = remove_displaced_original(original, unlinks.leftovers()).await
                else {
                    return Ok(());
                };
                let Some(progress) = progress else {
                    let name = target.file_name().unwrap_or_default().to_string_lossy();
                    return Err(io_error(original_not_removed(&name, &kept_in, &error)));
                };
                progress.report(TransferReport {
                    kept: vec![KeptItem {
                        location: Location::local(kept),
                        cause: KeptCause::OriginalNotRemoved(crate::services::gio_error_detail(
                            &error,
                        )),
                    }],
                    ..TransferReport::default()
                });
                Ok(())
            }
            Self::Aside(aside) => aside.remove(progress, unlinks.leftovers()).await,
        }
    }

    fn kept(&self, flush: &str) -> String {
        match self {
            Self::Sources(..) | Self::Source(..) => flush.to_owned(),
            Self::Displaced(original, target) => original_not_removed(
                &target.file_name().unwrap_or_default().to_string_lossy(),
                original.kept_in(),
                &io_error(flush),
            ),
            Self::Aside(aside) => aside.not_removed(&io_error(flush)).message().to_owned(),
        }
    }
}

#[derive(Default)]
pub(super) struct Owed {
    /// Opened before the copy wrote; none where only GIO can reach it.
    pub barrier: Option<Barrier>,
    pub nested: Vec<Barrier>,
    deletions: Vec<Deletion>,
}

impl Owed {
    pub(super) fn owe(&mut self, deletion: Deletion) {
        self.deletions.push(deletion);
    }

    /// Entries given back their names beneath the copy's root, deepest first, so the copy is
    /// still found before its sources are deleted.
    pub(super) fn follow_renames(&mut self, renamed: &[(PathBuf, PathBuf)]) {
        for deletion in &mut self.deletions {
            if let Deletion::Sources(_, manifest) = deletion {
                for (from, to) in renamed {
                    manifest.follow_rename(from, to);
                }
            }
        }
    }

    pub(super) async fn settle(
        self,
        progress: Option<&TransferProgressTracker>,
        cancellable: &gio::Cancellable,
    ) -> Result<(), glib::Error> {
        if self.deletions.is_empty() {
            return Ok(());
        }
        let mut batch = Batch::new(None);
        batch.add((), self, 0);
        batch
            .close(progress, &Arc::default(), cancellable)
            .await
            .pop()
            .map_or(Ok(()), |((), settled)| settled)
    }
}

pub(super) async fn flush(barrier: Barrier) -> Result<(), String> {
    let removable = !removable_roots_for_paths(std::slice::from_ref(&barrier.path))
        .await
        .is_empty();
    copy_engine::reach(barrier, removable)
        .await
        .map_err(|error| device_flush_failure(&error))
}

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub items: usize,
    pub bytes: u64,
    /// Time since its first entry, checked as items finish.
    pub age: Duration,
}

const LIMITS: Limits = Limits {
    items: 256,
    bytes: 1 << 30,
    age: Duration::from_secs(5),
};

#[cfg(test)]
thread_local! {
    static TEST_LIMITS: Cell<Option<Limits>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(super) fn set_batch_limits_for_test(limits: Option<Limits>) {
    TEST_LIMITS.set(limits);
}

fn limits() -> Limits {
    #[cfg(test)]
    if let Some(limits) = TEST_LIMITS.get() {
        return limits;
    }
    LIMITS
}

/// How far tests moved the batches' clock on, in milliseconds; racers on
/// worker threads move it.
#[cfg(test)]
static TEST_CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
pub(super) fn advance_batch_clock_for_test(by: Option<Duration>) {
    use std::sync::atomic::Ordering;
    match by {
        Some(by) => {
            let millis = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
            TEST_CLOCK.fetch_add(millis, Ordering::SeqCst);
        }
        None => TEST_CLOCK.store(0, Ordering::SeqCst),
    }
}

fn now() -> Instant {
    #[cfg(test)]
    {
        let advanced = TEST_CLOCK.load(std::sync::atomic::Ordering::SeqCst);
        Instant::now() + Duration::from_millis(advanced)
    }
    #[cfg(not(test))]
    {
        Instant::now()
    }
}

struct Batch<T> {
    key: Option<MountKey>,
    /// The earliest opened of its entries' barriers, so it reports the
    /// writeback errors of every one of their copies.
    barrier: Option<Barrier>,
    nested: Vec<Barrier>,
    entries: Vec<(T, Vec<Deletion>)>,
    started: Instant,
    copied: u64,
}

impl<T> Batch<T> {
    fn new(key: Option<MountKey>) -> Self {
        Self {
            key,
            barrier: None,
            nested: Vec::new(),
            entries: Vec::new(),
            started: now(),
            copied: 0,
        }
    }

    fn add(&mut self, tag: T, owed: Owed, copied: u64) {
        self.copied = self.copied.saturating_add(copied);
        if let Some(barrier) = owed.barrier {
            keep_earliest(&mut self.barrier, barrier);
        }
        for barrier in owed.nested {
            match self.nested.iter().position(|kept| kept.key == barrier.key) {
                Some(index) => {
                    let mut kept = Some(self.nested.swap_remove(index));
                    keep_earliest(&mut kept, barrier);
                    self.nested.extend(kept);
                }
                None => self.nested.push(barrier),
            }
        }
        self.entries.push((tag, owed.deletions));
    }

    fn due(&self) -> bool {
        let limits = limits();
        self.key.is_none()
            || self.entries.len() >= limits.items
            || self.copied >= limits.bytes
            || now().saturating_duration_since(self.started) >= limits.age
    }

    /// A failed flush deletes nothing.
    async fn close(
        self,
        progress: Option<&TransferProgressTracker>,
        unlinks: &Arc<Unlinks>,
        cancellable: &gio::Cancellable,
    ) -> Vec<(T, Result<(), glib::Error>)> {
        let mut flushed = Ok(());
        for barrier in self.barrier.into_iter().chain(self.nested) {
            flushed = flush(barrier).await;
            if flushed.is_err() {
                break;
            }
        }
        let mut settled = Vec::with_capacity(self.entries.len());
        for (tag, deletions) in self.entries {
            let result = match &flushed {
                Err(flush) => {
                    // Each other message names where an original stays, and
                    // includes `flush`.
                    let message = deletions
                        .iter()
                        .map(|deletion| deletion.kept(flush))
                        .filter(|message| message != flush)
                        .reduce(|first, second| crate::i18n::join_sentences(&first, &second))
                        .unwrap_or_else(|| flush.clone());
                    Err(io_error(message))
                }
                Ok(()) => {
                    let mut result = Ok(());
                    for deletion in deletions {
                        result = deletion.run(progress, unlinks, cancellable).await;
                        if result.is_err() {
                            break;
                        }
                    }
                    result
                }
            };
            settled.push((tag, result));
        }
        settled
    }
}

fn keep_earliest(kept: &mut Option<Barrier>, barrier: Barrier) {
    if kept.as_ref().is_none_or(|kept| barrier.precedes(kept)) {
        *kept = Some(barrier);
    }
}

struct Batches<T> {
    open: Vec<Batch<T>>,
}

impl<T> Batches<T> {
    fn add(&mut self, tag: T, owed: Owed, copied: u64) {
        let key = owed.barrier.as_ref().map(|barrier| barrier.key);
        let index = match self
            .open
            .iter()
            .position(|batch| key.is_some() && batch.key == key)
        {
            Some(index) => index,
            None => {
                self.open.push(Batch::new(key));
                self.open.len() - 1
            }
        };
        self.open[index].add(tag, owed, copied);
    }

    fn take(&mut self, mut close: impl FnMut(&Batch<T>) -> bool) -> Vec<Batch<T>> {
        let (closed, open) = std::mem::take(&mut self.open)
            .into_iter()
            .partition(|batch| close(batch));
        self.open = open;
        closed
    }
}

pub(super) struct Done {
    pub index: usize,
    pub moved_to: Option<Location>,
}

pub(super) struct Source {
    pub location: Location,
    pub merged: bool,
}

/// A transfer's items as they are done: an item that owes deletions is
/// done, and reported moved, only once its batch closes.
pub(super) struct Settling {
    request_id: OperationRequestId,
    sources: Vec<Source>,
    moving: bool,
    emit: Rc<dyn Fn(OperationEvent)>,
    progress: Rc<TransferProgressTracker>,
    batches: Batches<Done>,
    unlinks: Arc<Unlinks>,
    completed: Vec<usize>,
    failed: Vec<usize>,
    failures: Vec<String>,
}

impl Settling {
    pub(super) fn new(
        request_id: OperationRequestId,
        (sources, moving): (Vec<Source>, bool),
        emit: Rc<dyn Fn(OperationEvent)>,
        progress: Rc<TransferProgressTracker>,
    ) -> Self {
        Self {
            request_id,
            sources,
            moving,
            emit,
            progress,
            batches: Batches { open: Vec::new() },
            unlinks: Arc::default(),
            completed: Vec::new(),
            failed: Vec::new(),
            failures: Vec::new(),
        }
    }

    pub(super) fn paste(
        request: &PasteRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
        progress: Rc<TransferProgressTracker>,
    ) -> Self {
        let sources = request
            .items
            .iter()
            .map(|item| Source {
                location: item.source.clone(),
                merged: item.conflict == TransferConflict::Merge,
            })
            .collect();
        Self::new(request.id, (sources, request.move_sources), emit, progress)
    }

    fn done(&mut self, done: Done) {
        let source = &self.sources[done.index];
        if self.moving
            && let Some(to) = done.moved_to
            && to != source.location
        {
            (self.emit)(OperationEvent::ItemMoved {
                request_id: self.request_id,
                from: source.location.clone(),
                to,
                merged: source.merged,
            });
        }
        self.completed.push(done.index);
    }

    /// Batches that are due close, except after the `last` item: [`Self::finish`]
    /// closes them, showing the device flush.
    pub(super) async fn owe(&mut self, done: Done, owed: Owed, copied: u64, last: bool) {
        if owed.deletions.is_empty() {
            self.done(done);
            return;
        }
        self.batches.add(done, owed, copied);
        if !last {
            let due = self.batches.take(Batch::due);
            self.close(due).await;
        }
    }

    async fn close(&mut self, batches: Vec<Batch<Done>>) {
        // What complete copies owe is finished whatever stops the paste.
        let cancellable = gio::Cancellable::new();
        for batch in batches {
            let closed = batch
                .close(Some(&self.progress), &self.unlinks, &cancellable)
                .await;
            for (done, result) in closed {
                match result {
                    Ok(()) => self.done(done),
                    Err(error) => {
                        self.failed.push(done.index);
                        let message = crate::services::gio_error_message(&error);
                        if !self.failures.contains(&message) {
                            self.failures.push(message);
                        }
                    }
                }
            }
        }
    }

    /// Closes every batch; the last barrier shows as writing to the device.
    pub(super) async fn finish(&mut self) {
        let batches = self.batches.take(|_| true);
        let flushing = batches.iter().any(|batch| {
            batch
                .barrier
                .iter()
                .chain(&batch.nested)
                .any(Barrier::flushes)
        });
        if flushing {
            (self.emit)(OperationEvent::FlushingToDevice {
                request_id: self.request_id,
            });
        }
        self.close(batches).await;
    }

    pub(super) fn failure(&self) -> Option<String> {
        self.failures
            .iter()
            .cloned()
            .reduce(|first, second| crate::i18n::join_sentences(&first, &second))
    }

    fn locations(&self, indices: &[usize]) -> Vec<Location> {
        let mut indices = indices.to_vec();
        indices.sort_unstable();
        indices
            .into_iter()
            .map(|index| self.sources[index].location.clone())
            .collect()
    }

    pub(super) fn completed(&self) -> Vec<Location> {
        self.locations(&self.completed)
    }

    pub(super) fn failed(&self) -> Vec<Location> {
        self.locations(&self.failed)
    }
}
