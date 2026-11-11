// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone)]
pub(in crate::adapters::local_operations) struct ParallelCopyPlan {
    pub(in crate::adapters::local_operations) index: usize,
    pub(in crate::adapters::local_operations) source_path: PathBuf,
    pub(in crate::adapters::local_operations) target_location: Location,
    pub(in crate::adapters::local_operations) written_path: PathBuf,
    pub(in crate::adapters::local_operations) renamed: Option<RenameCause>,
}

/// What a paste planned, for deciding whether its items can be copied in parallel.
pub(in crate::adapters::local_operations) struct PastePlan<'a> {
    pub(in crate::adapters::local_operations) request: &'a PasteRequest,
    pub(in crate::adapters::local_operations) sources: &'a [gio::File],
    pub(in crate::adapters::local_operations) destination: &'a gio::File,
    pub(in crate::adapters::local_operations) planned: &'a [Option<Planned>],
    pub(in crate::adapters::local_operations) kinds: &'a [Option<gio::FileType>],
    pub(in crate::adapters::local_operations) windows_disk: bool,
}

pub(in crate::adapters::local_operations) async fn parallel_copy_plans(
    paste: PastePlan<'_>,
) -> Option<(Vec<ParallelCopyPlan>, usize)> {
    let PastePlan {
        request,
        sources,
        destination,
        planned,
        kinds,
        windows_disk,
    } = paste;
    if request.items.len() <= MAX_LOCAL_IO_WORKERS || !destination.is_native() {
        return None;
    }
    if request
        .items
        .iter()
        .any(|item| item.conflict != TransferConflict::FailIfExists)
    {
        return None;
    }
    let targets = regular_targets(planned, kinds, destination);
    let (sources, destination) = (sources.to_vec(), destination.clone());
    gio::spawn_blocking(move || {
        let plans = copy_plans(&sources, targets, &destination)?;
        let available = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let rotational = rotational(&plans, &destination, windows_disk);
        Some((plans, bounded_local_worker_count(available, rotational)))
    })
    .await
    .ok()
    .flatten()
}

type PlannedTarget = Option<(gio::File, Option<RenameCause>)>;

fn regular_targets(
    planned: &[Option<Planned>],
    kinds: &[Option<gio::FileType>],
    destination: &gio::File,
) -> Vec<PlannedTarget> {
    planned
        .iter()
        .zip(kinds)
        .map(|(planned, kind)| {
            planned
                .as_ref()
                .filter(|_| *kind == Some(gio::FileType::Regular))
                .map(|planned| (destination.child(&planned.target), planned.cause))
        })
        .collect()
}

fn copy_plans(
    sources: &[gio::File],
    targets: Vec<PlannedTarget>,
    destination: &gio::File,
) -> Option<Vec<ParallelCopyPlan>> {
    sources
        .iter()
        .zip(targets)
        .enumerate()
        .map(|(index, (source, target))| {
            let (target, renamed) = target.filter(|_| source.is_native())?;
            if transfer_is_noop(source, destination, &target) {
                return None;
            }
            Some(ParallelCopyPlan {
                index,
                source_path: source.path()?,
                target_location: location_for_file(&target)?,
                written_path: target.path()?,
                renamed,
            })
        })
        .collect()
}

/// A rotational disk on either side reads or writes one file at a time;
/// a disk Windows formats is taken for one where its device does not say.
fn rotational(plans: &[ParallelCopyPlan], destination: &gio::File, windows_disk: bool) -> bool {
    let source_folders: HashSet<&Path> = plans
        .iter()
        .filter_map(|plan| plan.source_path.parent())
        .collect();
    destination
        .path()
        .and_then(|path| std::fs::File::open(path).ok())
        .and_then(|directory| fs().rotational(directory.as_fd()))
        .unwrap_or(windows_disk)
        || source_folders.into_iter().any(|folder| {
            open_local_parent_directory(folder)
                .ok()
                .and_then(|folder| fs().rotational(folder.as_fd()))
                == Some(true)
        })
}

pub(in crate::adapters::local_operations) struct ParallelCopyOutcome {
    pub(in crate::adapters::local_operations) failed: Vec<(usize, glib::Error)>,
    pub(in crate::adapters::local_operations) next: usize,
    pub(in crate::adapters::local_operations) written_paths: Vec<PathBuf>,
}

/// Copies or moves planned regular files a chunk of workers at a time.
pub(in crate::adapters::local_operations) struct ParallelCopy<'a> {
    pub(in crate::adapters::local_operations) plans: &'a [ParallelCopyPlan],
    pub(in crate::adapters::local_operations) sizes: &'a TransferSizes,
    pub(in crate::adapters::local_operations) worker_count: usize,
    pub(in crate::adapters::local_operations) transferring: Transferring,
    pub(in crate::adapters::local_operations) settling: &'a mut Settling,
    pub(in crate::adapters::local_operations) variants: &'a mut CaseVariants,
}

/// How each worker transfers its file.
#[derive(Clone)]
pub(in crate::adapters::local_operations) struct Transferring {
    pub(in crate::adapters::local_operations) moving: bool,
    pub(in crate::adapters::local_operations) cancellable: gio::Cancellable,
    pub(in crate::adapters::local_operations) copying: Copying,
}

type Task = glib::JoinHandle<Result<(Transferred, Taken), glib::Error>>;

impl ParallelCopy<'_> {
    pub(in crate::adapters::local_operations) async fn run(mut self) -> ParallelCopyOutcome {
        let plans = self.plans;
        let mut outcome = ParallelCopyOutcome {
            failed: Vec::new(),
            next: 0,
            written_paths: Vec::new(),
        };
        while outcome.next < plans.len() && !self.transferring.cancellable.is_cancelled() {
            let start = outcome.next;
            let end = self.chunk_end(start);
            let chunk = &plans[start..end];
            outcome
                .written_paths
                .extend(chunk.iter().map(|plan| plan.written_path.clone()));
            let tasks = self.spawn(chunk);
            let renamed = self.collect(chunk, tasks, &mut outcome.failed).await;
            if let Some(progress) = &self.transferring.copying.progress {
                progress.report(TransferReport {
                    renamed,
                    ..TransferReport::default()
                });
            }
            outcome.next = end;
            if !outcome.failed.is_empty() || self.settling.failure().is_some() {
                break;
            }
        }
        outcome
    }

    /// An item waits for the earlier ones whose names differ from its own only in case.
    fn chunk_end(&self, start: usize) -> usize {
        let plans = self.plans;
        let full = (start + self.worker_count).min(plans.len());
        (start + 1..full)
            .find(|&position| {
                plans[start..position]
                    .iter()
                    .any(|earlier| self.variants.follows(plans[position].index, earlier.index))
            })
            .unwrap_or(full)
    }

    fn spawn(&self, chunk: &[ParallelCopyPlan]) -> Vec<Task> {
        let context = glib::MainContext::default();
        chunk
            .iter()
            .cloned()
            .map(|plan| {
                let (transferring, earlier) =
                    (self.transferring.clone(), self.variants.earlier(plan.index));
                context.spawn_local(async move {
                    let target = gio::File::for_path(&plan.written_path);
                    let taken = if case_variants::taken_by(&target, earlier).await {
                        Taken::Number
                    } else {
                        Taken::Fail
                    };
                    transfer_planned(&plan, taken, &transferring)
                        .await
                        .map(|transferred| (transferred, taken))
                })
            })
            .collect()
    }

    async fn collect(
        &mut self,
        chunk: &[ParallelCopyPlan],
        tasks: Vec<Task>,
        failed: &mut Vec<(usize, glib::Error)>,
    ) -> Vec<RenamedItem> {
        let mut renamed = Vec::new();
        for (plan, task) in chunk.iter().zip(tasks) {
            match task.await {
                Ok(Ok((transferred, taken))) => {
                    self.settle(plan, (transferred, taken), &mut renamed).await;
                }
                Ok(Err(error)) => failed.push((plan.index, error)),
                Err(_) => failed.push((plan.index, io_error("Copy worker failed"))),
            }
        }
        renamed
    }

    async fn settle(
        &mut self,
        plan: &ParallelCopyPlan,
        (transferred, taken): (Transferred, Taken),
        renamed: &mut Vec<RenamedItem>,
    ) {
        self.variants.saved(plan.index, &transferred.target).await;
        let aliased = taken == Taken::Number;
        renamed.extend(renamed_plan(plan, &transferred.target, aliased));
        let moving = self.transferring.moving;
        if let Some(progress) = &self.transferring.copying.progress {
            progress.finish_item(
                transferred.reported,
                self.sizes.sizes[plan.index],
                self.sizes.files[plan.index],
                (!moving)
                    .then(|| location_for_file(&transferred.target))
                    .flatten(),
            );
        }
        let done = Done {
            index: plan.index,
            moved_to: location_for_file(&transferred.target),
        };
        let copied = transferred.reported.bytes;
        let is_last = Some(plan.index) == self.plans.last().map(|plan| plan.index);
        self.settling
            .owe(done, transferred.owed, copied, is_last)
            .await;
    }
}

fn renamed_plan(plan: &ParallelCopyPlan, target: &gio::File, aliased: bool) -> Option<RenamedItem> {
    let cause = plan.renamed.or(aliased.then_some(RenameCause::Alias))?;
    let name = plan.source_path.file_name()?;
    let requested = gio::File::for_path(plan.written_path.with_file_name(name));
    let saved_as = if aliased {
        target.clone()
    } else {
        gio::File::for_path(&plan.written_path)
    };
    renamed_item(&requested, &saved_as, cause)
}

async fn transfer_planned(
    plan: &ParallelCopyPlan,
    taken: Taken,
    transferring: &Transferring,
) -> Result<Transferred, glib::Error> {
    let Transferring {
        moving,
        cancellable,
        copying,
    } = transferring;
    if *moving {
        let source = gio::File::for_path(&plan.source_path);
        let target = gio::File::for_path(&plan.written_path);
        return move_owing(
            source,
            target,
            Moving::new(taken, cancellable.clone(), copying.clone()),
        )
        .await;
    }
    let options = copy_engine::Options {
        names: copying.names.clone(),
        ..copy_engine::Options::default()
    };
    let copied = copy_engine::copy_local(
        plan.source_path.clone(),
        copy_engine::Destination::New(plan.written_path.clone(), taken),
        options,
        copying.progress.clone(),
        cancellable,
    )
    .await?;
    Ok(Transferred {
        target: gio::File::for_path(plan.written_path.with_file_name(copied.name)),
        reported: copied.reported,
        owed: Owed::default(),
    })
}
