// SPDX-License-Identifier: MIT

use super::mount_profile::MountProfile;
use super::*;

mod item;

pub(super) fn paste(request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let _task = glib::MainContext::default().spawn_local(run(request, emit, cancellable.clone()));
    operation_handle(cancellable)
}

async fn run(
    request: PasteRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
) {
    let Some((plan, state)) = prepare(request, emit, cancellable).await else {
        return;
    };
    let parallel = parallel_copy_plans(PastePlan {
        request: &plan.request,
        sources: &plan.sources,
        destination: &plan.destination,
        planned: &plan.planned,
        kinds: &plan.sizes.kinds,
        windows_disk: plan.profile.windows_disk(),
    })
    .await;
    match parallel {
        Some((plans, worker_count)) => run_parallel(&plan, state, &plans, worker_count).await,
        None => item::Sequential::new(&plan, state).await.run().await,
    }
}

/// What the paste worked out before it transfers anything. `operation` comes last so it
/// ends after everything that reports progress has gone.
struct Plan {
    request: PasteRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
    destination: gio::File,
    sources: Vec<gio::File>,
    names: NamePolicy,
    profile: MountProfile,
    sizes: TransferSizes,
    crossing: Vec<bool>,
    planned: Vec<Option<Planned>>,
    targets: Vec<Option<gio::File>>,
    copying: Copying,
    progress: Rc<TransferProgressTracker>,
    operation: Rc<ActiveOperation>,
}

/// What the paste changes as it goes.
struct State {
    settling: Settling,
    variants: CaseVariants,
    affected_locations: HashSet<Location>,
}

/// Emits why and returns `None` when the paste cannot start.
async fn prepare(
    request: PasteRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
) -> Option<(Plan, State)> {
    let destination = gio_file_for_location(&request.destination);
    let request = in_copy_order(request, &destination);
    let affected_locations = source_folders(&request);
    let moved = if request.move_sources {
        release::natives(request.items.iter().map(|item| &item.source))
    } else {
        Vec::new()
    };
    let operation = Rc::new(ActiveOperation::begin(release::parents(&moved)));
    operation.include_folder(&request.destination);
    let sources = request
        .items
        .iter()
        .map(|item| gio_file_for_location(&item.source))
        .collect::<Vec<_>>();
    let names = NamePolicy::new(NameMode::Fix);
    let pasted_into = names
        .destination(&destination, &sources, &cancellable)
        .await;
    let max_file_size = pasted_into.profile.max_file_size;
    let sizes = match transfer_sizes(&sources, &cancellable, max_file_size).await {
        Ok(sizes) => sizes,
        Err(error) => {
            report_unsized(&request, &emit, &error, affected_locations);
            return None;
        }
    };
    let rules = names.applied(pasted_into.profile.names, pasted_into.profile.strictness);
    let crossing = pasted_into.fixing(&sizes.filesystems);
    let duplicate = duplicates(&sources, &destination, request.move_sources);
    let planned = match planned_paste(
        &request.items,
        &sources,
        &rules,
        (&crossing, &duplicate, &sizes.kinds),
    ) {
        Ok(planned) => planned,
        Err(error) => {
            emit(OperationEvent::TransferFailed {
                request_id: request.id,
                completed_locations: Vec::new(),
                message: placement_reason(error),
            });
            return None;
        }
    };
    let targets = planned
        .iter()
        .map(|planned| {
            planned
                .as_ref()
                .map(|planned| destination.child(&planned.target))
        })
        .collect::<Vec<_>>();
    release_sources(
        &operation,
        (&request, &targets, &duplicate),
        moved,
        &cancellable,
    )
    .await;
    let variants = CaseVariants::new(
        planned
            .iter()
            .enumerate()
            .filter_map(|(index, planned)| Some((index, planned.as_ref()?.target.clone()))),
    );
    let progress =
        TransferProgressTracker::new(request.id, sizes.total, sizes.total_files, emit.clone());
    progress.emit();
    let settling = Settling::paste(&request, emit.clone(), progress.clone());
    let copying = Copying {
        progress: Some(progress.clone()),
        names: names.clone(),
        ..Copying::default()
    };
    let state = State {
        settling,
        variants,
        affected_locations,
    };
    let plan = Plan {
        request,
        emit,
        cancellable,
        destination,
        sources,
        names,
        profile: pasted_into.profile,
        sizes,
        crossing,
        planned,
        targets,
        copying,
        progress,
        operation,
    };
    Some((plan, state))
}

fn source_folders(request: &PasteRequest) -> HashSet<Location> {
    let mut affected_locations = HashSet::from([request.destination.clone()]);
    for parent in request.items.iter().filter_map(|item| item.source.parent()) {
        affected_locations.insert(parent);
    }
    affected_locations
}

fn report_unsized(
    request: &PasteRequest,
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
                .map(|item| item.source.clone())
                .collect(),
            affected_locations,
        ));
        return;
    }
    // Too large for FAT32, which the error says.
    emit(OperationEvent::TransferFailed {
        request_id: request.id,
        completed_locations: Vec::new(),
        message: error.message().to_owned(),
    });
}

fn targets_of<'a>(
    (request, targets, duplicate): (&'a PasteRequest, &'a [Option<gio::File>], &'a [bool]),
    conflict: TransferConflict,
) -> impl Iterator<Item = &'a gio::File> {
    request
        .items
        .iter()
        .zip(targets)
        .zip(duplicate)
        .filter(move |((item, _), duplicate)| item.conflict == conflict && !**duplicate)
        .filter_map(|((_, target), _)| target.as_ref())
}

async fn release_sources(
    operation: &ActiveOperation,
    planned: (&PasteRequest, &[Option<gio::File>], &[bool]),
    moved: Vec<PathBuf>,
    cancellable: &gio::Cancellable,
) {
    operation
        .include_trees(targets_of(planned, TransferConflict::Merge).filter_map(location_for_file));
    let replaced =
        targets_of(planned, TransferConflict::ReplaceExisting).filter_map(gio::File::path);
    operation
        .release(moved.into_iter().chain(replaced).collect(), cancellable)
        .await;
}

impl Plan {
    async fn stop(&self, written_paths: &[PathBuf], stop: TransferStop) {
        stop_transfer(written_paths, &self.emit, self.request.id, stop).await;
    }

    async fn finish(&self, written_paths: Vec<PathBuf>, state: State) {
        let completed = state.settling.completed();
        if !flush_removable_writes(
            written_paths,
            &self.cancellable,
            &self.emit,
            self.request.id,
            &completed,
            state.affected_locations,
        )
        .await
        {
            return;
        }
        (self.emit)(OperationEvent::Pasted {
            request_id: self.request.id,
            locations: completed,
        });
    }
}

async fn run_parallel(
    plan: &Plan,
    mut state: State,
    plans: &[ParallelCopyPlan],
    worker_count: usize,
) {
    for copy in plans {
        state
            .affected_locations
            .insert(plan.request.items[copy.index].source.clone());
        state
            .affected_locations
            .insert(copy.target_location.clone());
    }
    let outcome = ParallelCopy {
        plans,
        sizes: &plan.sizes,
        worker_count,
        transferring: Transferring {
            moving: plan.request.move_sources,
            cancellable: plan.cancellable.clone(),
            copying: plan.copying.clone(),
        },
        settling: &mut state.settling,
        variants: &mut state.variants,
    }
    .run()
    .await;
    state.settling.finish().await;
    let unfinished = !outcome.failed.is_empty() || outcome.next < plans.len();
    if unfinished || state.settling.failure().is_some() {
        stop_parallel(plan, state, &outcome).await;
        return;
    }
    plan.finish(outcome.written_paths, state).await;
}

async fn stop_parallel(plan: &Plan, state: State, outcome: &ParallelCopyOutcome) {
    let items = &plan.request.items;
    let (cut_off, failures): (Vec<_>, Vec<_>) = outcome
        .failed
        .iter()
        .partition(|(_, error)| was_cancelled(error));
    let mut failed = state.settling.failed();
    failed.extend(
        failures
            .iter()
            .map(|(index, _)| items[*index].source.clone()),
    );
    let failure_error = failures.first().map(|(_, error)| error);
    let (failure, skip_flush) = failure_error.map_or((None, false), |error| {
        copy_failure_on_fat32(error, plan.profile.max_file_size.is_some())
    });
    let stop = TransferStop {
        completed: state.settling.completed(),
        failed,
        not_attempted: cut_off
            .iter()
            .map(|(index, _)| &items[*index])
            .chain(&items[outcome.next..])
            .map(|item| item.source.clone())
            .collect(),
        affected_locations: state.affected_locations,
        failure: joined_failures(failure, state.settling.failure()),
        skip_flush,
    };
    plan.stop(&outcome.written_paths, stop).await;
}
