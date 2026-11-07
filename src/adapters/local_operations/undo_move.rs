// SPDX-License-Identifier: MIT

use std::ops::ControlFlow;

use super::*;
use crate::services::ItemIdentity;

mod replaced;
use replaced::ReplacedMoves;

pub(super) fn undo_move(request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let _task = glib::MainContext::default().spawn_local(run(request, emit, cancellable.clone()));
    operation_handle(cancellable)
}

async fn run(
    request: UndoMoveRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
) {
    if let Some(undo) = UndoMove::prepare(request, emit, cancellable).await {
        undo.run().await;
    }
}

/// Moves items back where they came from. `operation` ends after everything that reports
/// progress has gone.
struct UndoMove {
    settling: Settling,
    restored_paths: Vec<PathBuf>,
    affected_locations: HashSet<Location>,
    progress: Rc<TransferProgressTracker>,
    names: NamePolicy,
    sizes: TransferSizes,
    sources: Vec<gio::File>,
    replaced: Option<ReplacedMoves>,
    _operation: ActiveOperation,
    request: UndoMoveRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
}

enum Cleanup {
    Trashed(Option<ItemIdentity>),
    /// Trash is unsupported and the folder was left or removed in place.
    Kept,
    Cancelled,
    Failed(TrashError),
}

fn undo_folders(request: &UndoMoveRequest) -> HashSet<Location> {
    let mut affected_locations = HashSet::new();
    let moved = request
        .items
        .iter()
        .flat_map(|item| [&item.record.current, &item.record.original]);
    if request
        .items
        .iter()
        .any(|item| item.record.replaced.is_some())
    {
        affected_locations.insert(Location::uri("trash:///"));
    }
    for location in moved.chain(&request.cleanup_locations) {
        affected_locations.insert(location.clone());
        if let Some(parent) = location.parent() {
            affected_locations.insert(parent);
        }
    }
    affected_locations
}

fn report_unsized(
    request: &UndoMoveRequest,
    emit: &Rc<dyn Fn(OperationEvent)>,
    error: &glib::Error,
    affected_locations: HashSet<Location>,
) {
    if was_cancelled(error) {
        emit(cancelled_event(
            request.id,
            Vec::new(),
            Vec::new(),
            request
                .items
                .iter()
                .map(|item| item.record.current.clone())
                .collect(),
            affected_locations,
        ));
        return;
    }
    emit(OperationEvent::TransferFailed {
        request_id: request.id,
        completed_locations: Vec::new(),
        message: crate::services::gio_error_message(error),
    });
}

async fn release_undone(
    operation: &ActiveOperation,
    request: &UndoMoveRequest,
    moved: Vec<PathBuf>,
    cancellable: &gio::Cancellable,
) {
    let replaced = release::natives(
        request
            .items
            .iter()
            .filter(|item| item.conflict == TransferConflict::ReplaceExisting)
            .map(|item| &item.record.original),
    );
    let cleaned = release::natives(&request.cleanup_locations);
    operation
        .release(
            moved.into_iter().chain(replaced).chain(cleaned).collect(),
            cancellable,
        )
        .await;
}

fn report_trashed(
    (emit, request_id): (&Rc<dyn Fn(OperationEvent)>, OperationRequestId),
    location: &Location,
    original: ReplacedOriginal,
) {
    if let ReplacedOriginal::Trashed(Some(identity)) = original {
        emit(OperationEvent::ItemTrashed {
            request_id,
            location: location.clone(),
            identity,
        });
    }
}

impl UndoMove {
    /// Emits why and returns `None` when the undo cannot start.
    async fn prepare(
        request: UndoMoveRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
        cancellable: gio::Cancellable,
    ) -> Option<Self> {
        let affected_locations = undo_folders(&request);
        let replaced = match ReplacedMoves::prepare(&request, &cancellable).await {
            Ok(replaced) => replaced,
            Err(error) => {
                report_unsized(&request, &emit, &error, affected_locations);
                return None;
            }
        };
        let moved = release::natives(request.items.iter().map(|item| &item.record.current));
        let operation =
            ActiveOperation::begin(release::parents(&moved).into_iter().chain(release::parents(
                &release::natives(request.items.iter().map(|item| &item.record.original)),
            )));
        let sources = request
            .items
            .iter()
            .map(|item| gio_file_for_location(&item.record.current))
            .collect::<Vec<_>>();
        let sizes = match transfer_sizes(&sources, &cancellable, None).await {
            Ok(sizes) => sizes,
            Err(error) => {
                report_unsized(&request, &emit, &error, affected_locations);
                return None;
            }
        };
        release_undone(&operation, &request, moved, &cancellable).await;
        let names = NamePolicy::new(request.names);
        let progress =
            TransferProgressTracker::new(request.id, sizes.total, sizes.total_files, emit.clone());
        progress.emit();
        let undone = request
            .items
            .iter()
            .map(|item| owed::Source {
                location: item.record.current.clone(),
                merged: false,
            })
            .collect();
        let settling = Settling::new(request.id, (undone, true), emit.clone(), progress.clone());
        Some(Self {
            settling,
            restored_paths: Vec::new(),
            affected_locations,
            progress,
            names,
            sizes,
            sources,
            replaced,
            _operation: operation,
            request,
            emit,
            cancellable,
        })
    }

    async fn run(mut self) {
        for index in 0..self.request.items.len() {
            if self.cancellable.is_cancelled() || self.settling.failure().is_some() {
                self.stop_before(index).await;
                return;
            }
            if self.restore_item(index).await.is_break() {
                return;
            }
        }
        self.finish().await;
    }

    async fn restore_item(&mut self, index: usize) -> ControlFlow<()> {
        let source = self.sources[index].clone();
        let started = self.progress.reported();
        let original = &self.request.items[index].record.original;
        if let Some(path) = original.native_path().map(Path::to_path_buf) {
            self.restored_paths.push(path);
        }
        let target = gio_file_for_location(original);
        let undone = undone_when_cancelled(&source, &target);
        let mut owed = match self.transfer(index, source, target).await {
            Ok(owed) => owed,
            Err(error) => {
                self.stop_failed(index, undone, &error).await;
                return ControlFlow::Break(());
            }
        };
        let item = &self.request.items[index];
        owed.follow_renames(&restore_renamed(&item.record, &self.progress).await);
        let reported = self.progress.reported().since(started);
        self.progress.finish_item(
            reported,
            self.sizes.sizes[index],
            self.sizes.files[index],
            None,
        );
        let done = Done {
            index,
            moved_to: Some(item.record.original.clone()),
        };
        let last = index + 1 == self.request.items.len();
        self.settling.owe(done, owed, reported.bytes, last).await;
        ControlFlow::Continue(())
    }

    async fn transfer(
        &mut self,
        index: usize,
        source: gio::File,
        target: gio::File,
    ) -> Result<Owed, glib::Error> {
        let item = &self.request.items[index];
        let copying = Copying {
            progress: Some(self.progress.clone()),
            names: self.names.clone(),
            ..Copying::default()
        };
        if item.conflict != TransferConflict::ReplaceExisting {
            return move_owing(
                source,
                target,
                Moving::new(Taken::Fail, self.cancellable.clone(), copying),
            )
            .await
            .map(|moved| moved.owed);
        }
        let reporting = (&self.emit, self.request.id);
        let replacing = Replacing {
            move_source: true,
            cancellable: &self.cancellable,
            affected_locations: Some(&mut self.affected_locations),
            copying,
            on_replaced: &|original, _| report_trashed(reporting, &item.record.original, original),
        };
        replace_owing(source, target, replacing).await
    }

    fn currents(&self, from: usize) -> Vec<Location> {
        self.request.items[from..]
            .iter()
            .map(|item| item.record.current.clone())
            .collect()
    }

    async fn stop(&mut self, stop: TransferStop) {
        stop_transfer(&self.restored_paths, &self.emit, self.request.id, stop).await;
    }

    async fn stop_before(mut self, index: usize) {
        let failure = self.settle().await;
        let stop = TransferStop {
            completed: self.settling.completed(),
            failed: self.settling.failed(),
            not_attempted: self.currents(index),
            affected_locations: std::mem::take(&mut self.affected_locations),
            failure,
            skip_flush: false,
        };
        self.stop(stop).await;
    }

    async fn stop_failed(&mut self, index: usize, undone: bool, error: &glib::Error) {
        let settled_failure = self.settle().await;
        let failure = (!was_cancelled(error)).then(|| crate::services::gio_error_message(error));
        let untouched = undone && was_cancelled(error);
        let mut failed = self.settling.failed();
        if !untouched {
            failed.push(self.request.items[index].record.current.clone());
        }
        let stop = TransferStop {
            completed: self.settling.completed(),
            failed,
            not_attempted: self.currents(index + usize::from(!untouched)),
            affected_locations: std::mem::take(&mut self.affected_locations),
            failure: joined_failures(failure, settled_failure),
            skip_flush: false,
        };
        self.stop(stop).await;
    }

    async fn finish(mut self) {
        if let Some(failure) = self.settle().await {
            let stop = TransferStop {
                completed: self.settling.completed(),
                failed: self.settling.failed(),
                not_attempted: Vec::new(),
                affected_locations: std::mem::take(&mut self.affected_locations),
                failure: Some(failure),
                skip_flush: false,
            };
            self.stop(stop).await;
            return;
        }
        let completed = self.settling.completed();
        if !flush_removable_writes(
            std::mem::take(&mut self.restored_paths),
            &self.cancellable,
            &self.emit,
            self.request.id,
            &completed,
            self.affected_locations.clone(),
        )
        .await
        {
            return;
        }
        self.clean_up(completed).await;
    }

    async fn clean_up(self, completed: Vec<Location>) {
        for location in &self.request.cleanup_locations {
            match self.clean_up_one(location).await {
                Cleanup::Trashed(identity) => self.report_cleaned(location, identity),
                Cleanup::Kept => {}
                Cleanup::Cancelled => {
                    self.report_cleanup_cancelled(completed, location);
                    return;
                }
                Cleanup::Failed(error) => {
                    self.report_cleanup_failed(completed, location, &error);
                    return;
                }
            }
        }
        (self.emit)(OperationEvent::Pasted {
            request_id: self.request.id,
            locations: completed,
        });
    }

    async fn clean_up_one(&self, location: &Location) -> Cleanup {
        let file = gio_file_for_location(location);
        match trash_with_cleanup(&file, &self.cancellable).await {
            Ok(identity) => Cleanup::Trashed(identity),
            Err(error) if error.is_unsupported() => {
                let removed = remove_empty_directory(&file, &self.cancellable).await;
                if removed.is_err_and(|error| was_cancelled(&error)) {
                    Cleanup::Cancelled
                } else {
                    Cleanup::Kept
                }
            }
            Err(error) => Cleanup::Failed(error),
        }
    }

    fn report_cleaned(&self, location: &Location, identity: Option<ItemIdentity>) {
        if let Some(identity) = identity {
            (self.emit)(OperationEvent::ItemTrashed {
                request_id: self.request.id,
                location: location.clone(),
                identity,
            });
        }
    }

    fn report_cleanup_cancelled(&self, completed: Vec<Location>, location: &Location) {
        (self.emit)(cancelled_event(
            self.request.id,
            completed,
            Vec::new(),
            vec![location.clone()],
            self.affected_locations.clone(),
        ));
    }

    fn report_cleanup_failed(
        &self,
        completed: Vec<Location>,
        location: &Location,
        error: &TrashError,
    ) {
        if was_cancelled(&error.error) {
            self.report_cleanup_cancelled(completed, location);
            return;
        }
        (self.emit)(OperationEvent::TransferFailed {
            request_id: self.request.id,
            completed_locations: completed,
            message: crate::services::gio_error_message(&error.error),
        });
    }
}
